#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_error::rate_limit::{RateLimitCategory, RateLimitDimension};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use sha2::Digest as _;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_clients::clients::{
    CachedClients, ClientCredential, ClientDeployment, ClientKind, ClientLookup,
};
use crate::deployment_clients::credentials::{
    ClientsideCredentials, clientside_deployment, dynamic_litellm_params, is_clientside_credential,
};
use crate::deployment_clients::guard::{
    LimitSource, MaxParallelRequestsExceeded, MaxParallelRequestsLimit, calculate_parallel_limit,
    installable_limit,
};
use crate::deployment_clients::lookup::{
    credential_deployment, deployment_credentials, provider_credentials, sensitive_credential_hash,
};
use crate::deployment_clients::mismatch::credential_mismatch_warning;
use crate::deployment_clients::request::{DeploymentView, RequestContext, merged_tools};
use crate::deployment_identity::spec::DeploymentSpec;

const CLIENT_TTL_SECONDS: u64 = 3600;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn spec(
    model_name: &str,
    id: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm_params = JsonObject::from_iter([("model".to_owned(), text("openai/gpt-4o"))]);
    for (key, value) in params {
        litellm_params.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::from_iter([("id".to_owned(), text(id))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .expect("deployment")
}

fn catalog(specs: &[DeploymentSpec]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for built in specs {
        catalog.model_names.insert(built.model_name.clone());
        catalog.index_patterns(built).expect("pattern");
        catalog.push(built.clone());
    }
    catalog
}

fn limit(max_parallel_requests: i64) -> Arc<MaxParallelRequestsLimit> {
    Arc::new(MaxParallelRequestsLimit::new(
        max_parallel_requests,
        String::from("d1"),
        String::from("gpt"),
    ))
}

fn clock_at(millis: u64) -> Arc<Mutex<u64>> {
    Arc::new(Mutex::new(millis))
}

fn system_clock(shared: &Arc<Mutex<u64>>) -> Arc<dyn Fn() -> std::time::SystemTime + Send + Sync> {
    let shared = Arc::clone(shared);
    Arc::new(move || {
        let millis = *shared.lock().expect("clock");
        UNIX_EPOCH
            .checked_add(Duration::from_millis(millis))
            .unwrap_or(UNIX_EPOCH)
    })
}

// One assertion per observable outcome, so a wrong precedence, count or field fails the case by name.
#[rstest]
#[case::explicit_wins(Some(3), Some(5), Some(6000), Some(9), Some(3))]
#[case::rpm_beats_tpm(None, Some(2), Some(6000), Some(9), Some(2))]
#[case::tpm_formula(None, None, Some(6000), Some(9), Some(36))]
#[case::tpm_rounds_up_from_zero(None, None, Some(166), Some(9), Some(1))]
#[case::tpm_just_over_one(None, None, Some(167), Some(9), Some(1))]
#[case::default_last(None, None, None, Some(9), Some(9))]
#[case::nothing_configured(None, None, None, None, None)]
#[case::explicit_zero_is_kept(Some(0), Some(5), None, Some(9), Some(0))]
fn deployment_clients_behaviors_calculate(
    #[case] max_parallel_requests: Option<i64>,
    #[case] rpm: Option<i64>,
    #[case] tpm: Option<i64>,
    #[case] default_max_parallel_requests: Option<i64>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(
        calculate_parallel_limit(
            max_parallel_requests,
            rpm,
            tpm,
            default_max_parallel_requests
        ),
        expected
    );
}

#[test]
fn deployment_clients_behaviors_install_a_zero_cap_never_caps() {
    let source = LimitSource {
        max_parallel_requests: Some(0),
        rpm: None,
        tpm: None,
        default_max_parallel_requests: None,
    };
    assert!(installable_limit(source, "d1", "gpt").is_none());
    let negative = LimitSource {
        max_parallel_requests: Some(-2),
        rpm: None,
        tpm: None,
        default_max_parallel_requests: None,
    };
    assert!(installable_limit(negative, "d1", "gpt").is_some());
}

#[test]
fn deployment_clients_behaviors_a_free_slot_is_granted_and_released_on_drop() {
    let limit = limit(1);
    let slot = limit.acquire().expect("a free slot");
    assert_eq!(limit.in_flight(), 1);
    drop(slot);
    assert_eq!(limit.in_flight(), 0);
}

#[test]
fn deployment_clients_behaviors_enter_and_exit_release_after_a_failure() {
    let limit = limit(1);
    let outcome = limit.with_slot(|| Err::<(), &str>("the provider failed"));
    assert_eq!(outcome, Ok(Err("the provider failed")));
    assert_eq!(limit.in_flight(), 0);
    assert!(limit.acquire().is_ok());
}

#[test]
fn deployment_clients_behaviors_a_full_limit_reports_the_concurrent_requests_error() {
    let limit = limit(1);
    let _held = limit.acquire().expect("a free slot");
    let error = limit.acquire().err().expect("the only slot is in use");
    assert_eq!(error.model_id, "d1");
    assert_eq!(error.model_group, "gpt");
    assert_eq!(error.max_parallel_requests, 1);
    assert_eq!(MaxParallelRequestsExceeded::STATUS_CODE, 429);
    assert_eq!(MaxParallelRequestsExceeded::CODE, "429");
    assert_eq!(MaxParallelRequestsExceeded::ERROR_TYPE, "throttling_error");
    assert_eq!(
        MaxParallelRequestsExceeded::CATEGORY,
        RateLimitCategory::LitellmRateLimit
    );
    assert_eq!(
        MaxParallelRequestsExceeded::DIMENSION,
        RateLimitDimension::ConcurrentRequests
    );
    assert!(error.to_string().contains("max_parallel_requests=1"));
}

#[test]
fn deployment_clients_behaviors_a_cached_limit_is_reused_for_the_same_model_id() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let source = LimitSource {
        max_parallel_requests: Some(2),
        rpm: None,
        tpm: None,
        default_max_parallel_requests: None,
    };
    let first = clients.max_parallel_requests("d1", "gpt", source);
    let second = clients.max_parallel_requests("d1", "gpt", source);
    assert!(first.initialized);
    assert!(!second.initialized);
    let (first_limit, second_limit) = (
        first.limit.expect("a limit"),
        second.limit.expect("a limit"),
    );
    assert!(Arc::ptr_eq(&first_limit, &second_limit));
}

#[test]
fn deployment_clients_behaviors_cached_limits_expire_after_the_client_ttl() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let source = LimitSource {
        max_parallel_requests: None,
        rpm: Some(3),
        tpm: None,
        default_max_parallel_requests: None,
    };
    let first = clients
        .max_parallel_requests("d1", "gpt", source)
        .limit
        .expect("a limit");
    *clock.lock().expect("clock") = 1;
    let second = clients
        .max_parallel_requests("d1", "gpt", source)
        .limit
        .expect("a limit");
    assert!(Arc::ptr_eq(&first, &second));
    *clock.lock().expect("clock") = (CLIENT_TTL_SECONDS + 1).saturating_mul(1000);
    let third = clients
        .max_parallel_requests("d1", "gpt", source)
        .limit
        .expect("a limit");
    assert!(!Arc::ptr_eq(&first, &third));
}

#[test]
fn deployment_clients_behaviors_no_configured_limit_leaves_the_cache_empty() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let lookup = clients.max_parallel_requests("d1", "gpt", LimitSource::default());
    assert!(lookup.initialized);
    assert!(lookup.limit.is_none());
}

#[rstest]
#[case::sync_plain(ClientKind::Sync, false, "d1_client")]
#[case::sync_stream(ClientKind::Sync, true, "d1_stream_client")]
#[case::async_plain(ClientKind::Async, false, "d1_async_client")]
#[case::async_stream(ClientKind::Async, true, "d1_stream_async_client")]
fn deployment_clients_behaviors_client_keys_follow_the_async_and_stream_choice(
    #[case] kind: ClientKind,
    #[case] stream: bool,
    #[case] key: &str,
) {
    assert_eq!(kind.cache_key("d1", stream), key);
}

#[test]
fn deployment_clients_behaviors_the_configured_client_is_reused_until_it_expires() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let lookup = ClientLookup {
        model_id: "d1",
        stream: false,
        api_key: None,
    };
    let deployment = ClientDeployment {
        model_group: "gpt",
        api_base: "https://configured.example/v1",
    };
    let first = clients.openai_client(ClientKind::Async, &lookup, &deployment);
    let second = clients.openai_client(ClientKind::Async, &lookup, &deployment);
    assert_eq!(first, second);
    assert_eq!(first.credential, ClientCredential::Configured);
    assert_eq!(first.api_base, "https://configured.example/v1");
    *clock.lock().expect("clock") = (CLIENT_TTL_SECONDS + 1).saturating_mul(1000);
    let third = clients.openai_client(ClientKind::Async, &lookup, &deployment);
    assert_eq!(third, first);
}

#[test]
fn deployment_clients_behaviors_a_request_key_never_reuses_the_configured_client() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let deployment = ClientDeployment {
        model_group: "gpt",
        api_base: "https://configured.example/v1",
    };
    let configured = clients.openai_client(
        ClientKind::Async,
        &ClientLookup {
            model_id: "d1",
            stream: false,
            api_key: None,
        },
        &deployment,
    );
    let caller_key = SecretString::from("sk-caller");
    let overridden = clients.openai_client(
        ClientKind::Async,
        &ClientLookup {
            model_id: "d1",
            stream: false,
            api_key: Some(&caller_key),
        },
        &deployment,
    );
    assert_ne!(configured.credential, overridden.credential);
    assert_eq!(
        overridden.credential,
        ClientCredential::Overridden(hex::encode(sha2::Sha256::digest(b"sk-caller")))
    );
    assert_eq!(overridden.api_base, "https://configured.example/v1");
}

#[rstest]
#[case::key(serde_json::json!({"api_key": "sk-a"}), true)]
#[case::base(serde_json::json!({"api_base": "https://caller.example/v1"}), true)]
#[case::base_url(serde_json::json!({"base_url": "https://caller.example/v1"}), true)]
#[case::null_still_counts(serde_json::json!({"api_key": null}), true)]
#[case::unrelated(serde_json::json!({"temperature": 0}), false)]
#[case::empty(serde_json::json!({}), false)]
fn deployment_clients_behaviors_clientside_detection_reads_presence(
    #[case] request: serde_json::Value,
    #[case] expected: bool,
) {
    let kwargs: JsonObject = match request {
        serde_json::Value::Object(fields) => fields
            .into_iter()
            .map(|(key, value)| (key, serde_json::from_value(value).expect("json")))
            .collect(),
        _ => JsonObject::new(),
    };
    assert_eq!(is_clientside_credential(&kwargs), expected);
}

#[test]
fn deployment_clients_behaviors_dynamic_params_take_only_the_request_key() {
    let config = JsonObject::from_iter([
        ("model".to_owned(), text("openai/gpt-4o")),
        ("api_base".to_owned(), text("https://configured.example/v1")),
    ]);
    let request = ClientsideCredentials {
        api_key: Some(SecretString::from("sk-caller")),
        api_base: Some(String::from("https://caller.example/v1")),
        base_url: Some(String::from("https://caller.example/v1")),
    };
    let dynamic = dynamic_litellm_params(config, &request);
    assert_eq!(dynamic.get("api_key"), Some(&text("sk-caller")));
    assert_eq!(
        dynamic.get("api_base"),
        Some(&text("https://configured.example/v1"))
    );
    assert!(!dynamic.contains_key("base_url"));
}

#[test]
fn deployment_clients_behaviors_a_clientside_deployment_gets_a_generated_id() {
    let configured = spec("gpt", "d1", &[("api_key", text("sk-config"))], &[]);
    let request = ClientsideCredentials {
        api_key: Some(SecretString::from("sk-caller")),
        api_base: None,
        base_url: None,
    };
    let per_request = clientside_deployment(&configured, "gpt", &request).expect("deployment");
    let deployment = &per_request.deployment;
    assert_eq!(deployment.model_name, "gpt");
    assert_eq!(
        deployment.model_info.get("original_model_id"),
        Some(&text("d1"))
    );
    let id = deployment.id().expect("a generated id");
    assert_ne!(id, "d1");
    let dynamic = dynamic_litellm_params(configured.litellm_params.clone(), &request);
    assert_eq!(
        id,
        crate::deployment_identity::stable_id::generate_model_id("gpt", &dynamic)
    );
    assert!(!deployment.litellm_params.contains_key("api_key"));
    assert!(!format!("{per_request:?}").contains("sk-caller"));
    assert_eq!(
        per_request
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-caller")
    );
    let repeated = clientside_deployment(&configured, "gpt", &request).expect("deployment");
    assert_eq!(repeated.deployment.id(), Some(id));
}

#[test]
fn deployment_clients_behaviors_a_request_without_a_key_keeps_the_configured_key() {
    let configured = spec("gpt", "d1", &[("api_key", text("sk-config"))], &[]);
    let request = ClientsideCredentials {
        api_key: None,
        api_base: Some(String::from("https://caller.example/v1")),
        base_url: None,
    };
    let per_request = clientside_deployment(&configured, "gpt", &request).expect("deployment");
    assert_eq!(
        per_request.deployment.litellm_params.get("api_key"),
        Some(&text("sk-config"))
    );
    assert!(per_request.api_key.is_none());
}

#[test]
fn deployment_clients_behaviors_tools_merge_supplier_first() {
    let deployment_tools = vec![text("deployment")];
    let request_tools = vec![text("request")];
    assert_eq!(
        merged_tools(&deployment_tools, &request_tools),
        Some(vec![text("deployment"), text("request")])
    );
    assert_eq!(merged_tools(&[], &[]), None);
    assert_eq!(
        merged_tools(&deployment_tools, &[]),
        Some(vec![text("deployment")])
    );
}

#[test]
fn deployment_clients_behaviors_the_request_update_writes_metadata_and_tags() {
    let mut info = JsonObject::new();
    info.insert("id".to_owned(), text("d1"));
    let tags = vec![String::from("deployment-tag")];
    let tools = vec![text("deployment-tool")];
    let mut context = RequestContext::default();
    context.update(&DeploymentView {
        model_name: "gpt",
        model: "openai/gpt-4o",
        api_base: Some(&text("https://configured.example/v1")),
        model_info: &info,
        tags: &tags,
        credential_name: Some("team-key"),
        tools: &tools,
        tool_choice: Some(&text("auto")),
    });
    assert_eq!(
        context.metadata.get("deployment"),
        Some(&text("openai/gpt-4o"))
    );
    assert_eq!(
        context.metadata.get("deployment_model_name"),
        Some(&text("gpt"))
    );
    assert_eq!(context.tool_choice, Some(text("auto")));
    assert_eq!(context.tools, Some(vec![text("deployment-tool")]));
    assert_eq!(
        context.tags,
        vec![
            String::from("deployment-tag"),
            String::from("Credential: team-key")
        ]
    );
    assert_eq!(context.model_info.get("id"), Some(&text("d1")));
    context.update(&DeploymentView {
        model_name: "gpt",
        model: "openai/gpt-4o",
        api_base: None,
        model_info: &info,
        tags: &tags,
        credential_name: Some("team-key"),
        tools: &[],
        tool_choice: None,
    });
    assert_eq!(context.tags.len(), 2);
    assert_eq!(context.metadata.get("api_base"), Some(&Json::Null));
}

#[test]
fn deployment_clients_behaviors_credentials_come_from_the_exact_id() {
    let built = spec(
        "gpt",
        "d1",
        &[
            ("api_key", text("sk-config")),
            ("model", text("openai/gpt-4o")),
        ],
        &[],
    );
    let catalog = catalog(&[built]);
    let credentials = deployment_credentials(&catalog, "d1")
        .expect("lookup")
        .expect("credentials");
    assert_eq!(credentials.get("api_key"), Some(&text("sk-config")));
    assert_eq!(credentials.len(), 1);
    assert!(
        deployment_credentials(&catalog, "missing")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn deployment_clients_behaviors_a_paused_deployment_hides_its_credentials() {
    let built = spec(
        "gpt",
        "d1",
        &[("api_key", text("sk-config"))],
        &[("blocked", Json::Bool(true))],
    );
    let catalog = catalog(&[built]);
    assert!(
        deployment_credentials(&catalog, "d1")
            .expect("lookup")
            .is_none()
    );
    assert!(credential_deployment(&catalog, "d1", None).is_none());
}

#[test]
fn deployment_clients_behaviors_the_provider_call_gets_model_and_provider() {
    let built = spec(
        "gpt",
        "d1",
        &[
            ("model", text("openai/gpt-4o")),
            ("api_key", text("sk-config")),
        ],
        &[],
    );
    let catalog = catalog(&[built]);
    let credentials =
        provider_credentials(&catalog, "d1", None, &std::collections::BTreeMap::new())
            .expect("lookup")
            .expect("credentials");
    assert_eq!(credentials.get("model"), Some(&text("openai/gpt-4o")));
    assert_eq!(
        credentials.get("custom_llm_provider"),
        Some(&text("openai"))
    );
    let prefixed = spec(
        "claude",
        "d2",
        &[("model", text("anthropic/claude-sonnet-5"))],
        &[],
    );
    let catalog = catalog(&[prefixed]);
    let credentials =
        provider_credentials(&catalog, "d2", None, &std::collections::BTreeMap::new())
            .expect("lookup")
            .expect("credentials");
    assert_eq!(
        credentials.get("custom_llm_provider"),
        Some(&text("anthropic"))
    );
}

#[test]
fn deployment_clients_behaviors_a_named_credential_is_resolved_and_dropped() {
    let built = spec(
        "gpt",
        "d1",
        &[
            ("model", text("openai/gpt-4o")),
            ("litellm_credential_name", text("team-key")),
        ],
        &[],
    );
    let catalog = catalog(&[built]);
    let mut stored = std::collections::BTreeMap::new();
    stored.insert(
        String::from("team-key"),
        JsonObject::from_iter([("api_key".to_owned(), text("sk-stored"))]),
    );
    let credentials = provider_credentials(&catalog, "d1", None, &stored)
        .expect("lookup")
        .expect("credentials");
    assert_eq!(credentials.get("api_key"), Some(&text("sk-stored")));
    assert!(!credentials.contains_key("litellm_credential_name"));
}

#[test]
fn deployment_clients_behaviors_the_credential_hash_ignores_non_credential_fields() {
    let first = JsonObject::from_iter([
        ("model".to_owned(), text("openai/gpt-4o")),
        ("api_key".to_owned(), text("sk-a")),
    ]);
    let second = JsonObject::from_iter([
        ("model".to_owned(), text("openai/other")),
        ("api_key".to_owned(), text("sk-a")),
    ]);
    let other = JsonObject::from_iter([("api_key".to_owned(), text("sk-b"))]);
    assert_eq!(
        sensitive_credential_hash(&first).expect("hash"),
        sensitive_credential_hash(&second).expect("hash")
    );
    assert_ne!(
        sensitive_credential_hash(&first).expect("hash"),
        sensitive_credential_hash(&other).expect("hash")
    );
}

#[test]
fn deployment_clients_behaviors_the_credential_warning_names_the_missing_prefix() {
    let params = JsonObject::from_iter([
        ("model".to_owned(), text("claude-sonnet-5")),
        ("aws_region_name".to_owned(), text("us-east-1")),
    ]);
    let warning =
        credential_mismatch_warning("claude", &params, Some("anthropic")).expect("warning");
    assert!(warning.contains("aws_region_name"));
    assert!(warning.contains("bedrock/"));
    assert!(credential_mismatch_warning("claude", &params, Some("bedrock")).is_none());
    assert!(credential_mismatch_warning("claude", &params, None).is_none());
    let vertex = JsonObject::from_iter([
        ("model".to_owned(), text("gemini-pro")),
        ("vertex_project".to_owned(), text("proj")),
    ]);
    assert!(credential_mismatch_warning("gemini", &vertex, Some("vertex_ai")).is_none());
    assert!(credential_mismatch_warning("gemini", &vertex, Some("openai")).is_some());
}

// Bounds, error fields and exact selection: the values just inside and just outside each limit.
#[rstest]
#[case::at_the_limit(1, 1, true)]
#[case::under_the_limit(2, 1, true)]
#[case::over_the_limit(1, 0, false)]
#[case::zero_admits_nothing(0, 0, false)]
#[case::negative_admits_nothing(-1, 0, false)]
fn deployment_clients_boundaries_acquire_compares_against_the_cap(
    #[case] max_parallel_requests: i64,
    #[case] in_flight: i64,
    #[case] granted: bool,
) {
    let limit = limit(max_parallel_requests);
    let held: Vec<_> = (0..in_flight)
        .map(|_| limit.acquire().expect("within the cap"))
        .collect();
    let acquired = limit.acquire();
    let granted_now = acquired.is_ok();
    assert_eq!(granted_now, granted);
    assert_eq!(limit.in_flight(), in_flight + i64::from(granted_now));
    drop(acquired);
    drop(held);
    assert_eq!(limit.in_flight(), 0);
}

#[rstest]
#[case::tpm_zero(Some(0), Some(1))]
#[case::tpm_negative_rounds_to_one(Some(-150), Some(1))]
#[case::tpm_negative_beyond_one(Some(-1500), Some(-9))]
#[case::tpm_exact_multiple(Some(5000), Some(30))]
fn deployment_clients_boundaries_tpm_truncates_like_the_azure_formula(
    #[case] tpm: Option<i64>,
    #[case] expected: Option<i64>,
) {
    assert_eq!(calculate_parallel_limit(None, None, tpm, None), expected);
}

#[test]
fn deployment_clients_boundaries_an_id_beats_a_name_and_a_name_beats_a_wildcard() {
    let by_id = spec("shared", "d1", &[("api_key", text("sk-id"))], &[]);
    let by_name = spec("shared", "d2", &[("api_key", text("sk-name"))], &[]);
    let wildcard = spec("openai/*", "d3", &[("api_key", text("sk-wild"))], &[]);
    let catalog = catalog(&[by_name, wildcard, by_id]);
    let chosen = credential_deployment(&catalog, "d1", None).expect("by id");
    assert_eq!(chosen.id(), Some("d1"));
    let chosen = credential_deployment(&catalog, "shared", None).expect("by name");
    assert_eq!(chosen.id(), Some("d2"));
    let chosen = credential_deployment(&catalog, "openai/gpt-4o", None).expect("wildcard");
    assert_eq!(chosen.id(), Some("d3"));
}

#[test]
fn deployment_clients_boundaries_another_teams_deployment_is_not_resolved_by_name() {
    let team_deployment = spec(
        "shared",
        "d1",
        &[("api_key", text("sk-team"))],
        &[("team_id", text("team-a"))],
    );
    let catalog = catalog(&[team_deployment]);
    assert!(credential_deployment(&catalog, "shared", None).is_none());
    assert!(credential_deployment(&catalog, "shared", Some("team-b")).is_none());
    let chosen = credential_deployment(&catalog, "shared", Some("team-a")).expect("own team");
    assert_eq!(chosen.id(), Some("d1"));
    assert!(credential_deployment(&catalog, "d1", Some("team-b")).is_some());
}

#[test]
fn deployment_clients_boundaries_a_team_wildcard_shadows_a_global_one() {
    let global = spec("openai/*", "g1", &[("api_key", text("sk-global"))], &[]);
    let team = spec(
        "openai/*",
        "t1",
        &[("api_key", text("sk-team"))],
        &[
            ("team_id", text("team-a")),
            ("team_public_model_name", text("openai/*")),
        ],
    );
    let catalog = catalog(&[global, team]);
    let chosen = credential_deployment(&catalog, "openai/gpt-4o", Some("team-a")).expect("team");
    assert_eq!(chosen.id(), Some("t1"));
    let chosen = credential_deployment(&catalog, "openai/gpt-4o", Some("team-b")).expect("global");
    assert_eq!(chosen.id(), Some("g1"));
    let chosen = credential_deployment(&catalog, "openai/gpt-4o", None).expect("global");
    assert_eq!(chosen.id(), Some("g1"));
}

#[test]
fn deployment_clients_boundaries_a_cached_client_expires_exactly_at_the_client_ttl() {
    let clock = clock_at(0);
    let clients = CachedClients::new(
        Duration::from_secs(CLIENT_TTL_SECONDS),
        system_clock(&clock),
    );
    let lookup = ClientLookup {
        model_id: "d1",
        stream: false,
        api_key: None,
    };
    let first = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://a.example/v1",
        },
    );
    assert_eq!(first.api_base, "https://a.example/v1");
    *clock.lock().expect("clock") = 1;
    let fresh = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://b.example/v1",
        },
    );
    assert_eq!(fresh.api_base, "https://a.example/v1");
    *clock.lock().expect("clock") = CLIENT_TTL_SECONDS.saturating_mul(1000);
    let expired = clients.openai_client(
        ClientKind::Async,
        &lookup,
        &ClientDeployment {
            model_group: "gpt",
            api_base: "https://c.example/v1",
        },
    );
    assert_eq!(expired.api_base, "https://c.example/v1");
}

#[test]
fn deployment_clients_boundaries_a_null_credential_field_is_dropped() {
    let built = spec("gpt", "d1", &[("api_key", Json::Null)], &[]);
    let catalog = catalog(&[built]);
    let credentials = deployment_credentials(&catalog, "d1")
        .expect("lookup")
        .expect("credentials");
    assert!(!credentials.contains_key("api_key"));
    assert!(credentials.is_empty());
}

#[test]
fn deployment_clients_boundaries_a_declared_provider_wins_over_the_model_prefix() {
    let built = spec(
        "claude",
        "d1",
        &[
            ("model", text("anthropic/claude-sonnet-5")),
            ("custom_llm_provider", text("bedrock")),
        ],
        &[],
    );
    let catalog = catalog(&[built]);
    let credentials =
        provider_credentials(&catalog, "d1", None, &std::collections::BTreeMap::new())
            .expect("lookup")
            .expect("credentials");
    assert_eq!(
        credentials.get("custom_llm_provider"),
        Some(&text("bedrock"))
    );
}

#[test]
fn deployment_clients_boundaries_an_empty_update_writes_no_tags() {
    let info = JsonObject::new();
    let mut context = RequestContext::default();
    context.update(&DeploymentView {
        model_name: "gpt",
        model: "openai/gpt-4o",
        api_base: None,
        model_info: &info,
        tags: &[],
        credential_name: None,
        tools: &[],
        tool_choice: None,
    });
    assert!(!context.metadata.contains_key("tags"));
    assert!(context.tags.is_empty());
}

#[test]
fn deployment_clients_boundaries_a_credential_field_of_the_wrong_type_is_an_error() {
    let flag = spec("gpt", "d1", &[("api_key", Json::Bool(true))], &[]);
    let error = deployment_credentials(&catalog(&[flag]), "d1").expect_err("a bool is no key");
    assert!(error.to_string().contains("api_key"), "{error}");
    let list = spec("gpt", "d2", &[("api_key", Json::List(Vec::new()))], &[]);
    assert!(deployment_credentials(&catalog(&[list]), "d2").is_err());
    let object = spec(
        "gpt",
        "d3",
        &[("vertex_credentials", Json::Object(JsonObject::new()))],
        &[],
    );
    assert!(deployment_credentials(&catalog(&[object]), "d3").is_ok());
    let tags = spec(
        "gpt",
        "d4",
        &[("bedrock_tags", Json::List(Vec::new()))],
        &[],
    );
    assert!(deployment_credentials(&catalog(&[tags]), "d4").is_ok());
}
