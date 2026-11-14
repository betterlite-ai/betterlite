use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_error::rate_limit::{RateLimitCategory, RateLimitDimension};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use sha2::Digest as _;

use super::{CLIENT_TTL_SECONDS, catalog, clock_at, limit, spec, system_clock, text};
use crate::deployment_clients::clients::{
    CachedClients, ClientCredential, ClientDeployment, ClientKind, ClientLookup,
};
use crate::deployment_clients::credentials::{
    ClientsideCredentials, clientside_deployment, dynamic_litellm_params, is_clientside_credential,
};
use crate::deployment_clients::guard::{
    LimitSource, MaxParallelRequestsExceeded, calculate_parallel_limit, installable_limit,
};
use crate::deployment_clients::lookup::{
    credential_deployment, deployment_credentials, provider_credentials, sensitive_credential_hash,
};
use crate::deployment_clients::mismatch::credential_mismatch_warning;
use crate::deployment_clients::request::{DeploymentView, RequestContext, merged_tools};

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
fn calculate(
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
fn install_a_zero_cap_never_caps() {
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
fn a_free_slot_is_granted_and_released_on_drop() {
    let limit = limit(1);
    let slot = limit.acquire().expect("a free slot");
    assert_eq!(limit.in_flight.load(Ordering::SeqCst), 1);
    drop(slot);
    assert_eq!(limit.in_flight.load(Ordering::SeqCst), 0);
}

#[test]
fn enter_and_exit_release_after_a_failure() {
    let limit = limit(1);
    let outcome: Result<(), &str> = {
        let _held = limit.acquire().expect("a free slot");
        Err("the provider failed")
    };
    assert_eq!(outcome, Err("the provider failed"));
    assert_eq!(limit.in_flight.load(Ordering::SeqCst), 0);
    assert!(limit.acquire().is_ok());
}

#[test]
fn a_full_limit_reports_the_concurrent_requests_error() {
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
fn a_cached_limit_is_reused_for_the_same_model_id() {
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
fn cached_limits_expire_after_the_client_ttl() {
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
fn no_configured_limit_leaves_the_cache_empty() {
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
fn client_keys_follow_the_async_and_stream_choice(
    #[case] kind: ClientKind,
    #[case] stream: bool,
    #[case] key: &str,
) {
    assert_eq!(kind.cache_key("d1", stream), key);
}

#[test]
fn the_configured_client_is_reused_until_it_expires() {
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
fn a_request_key_never_reuses_the_configured_client() {
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
fn clientside_detection_reads_presence(#[case] request: serde_json::Value, #[case] expected: bool) {
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
fn dynamic_params_take_only_the_request_key() {
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
fn a_clientside_deployment_gets_a_generated_id() {
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
fn a_request_without_a_key_keeps_the_configured_key() {
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
fn tools_merge_supplier_first() {
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
fn the_request_update_writes_metadata_and_tags() {
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
}

#[test]
fn a_second_update_adds_no_tag_twice_and_writes_a_null_api_base() {
    let info = JsonObject::new();
    let tags = vec![String::from("deployment-tag")];
    let mut context = RequestContext::default();
    let view = DeploymentView {
        model_name: "gpt",
        model: "openai/gpt-4o",
        api_base: None,
        model_info: &info,
        tags: &tags,
        credential_name: Some("team-key"),
        tools: &[],
        tool_choice: None,
    };
    context.update(&view);
    context.update(&view);
    assert_eq!(context.tags.len(), 2);
    assert_eq!(context.metadata.get("api_base"), Some(&Json::Null));
}

#[test]
fn credentials_come_from_the_exact_id() {
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
fn a_paused_deployment_hides_its_credentials() {
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
fn the_provider_call_gets_model_and_provider() {
    let built = spec(
        "gpt",
        "d1",
        &[
            ("model", text("openai/gpt-4o")),
            ("api_key", text("sk-config")),
        ],
        &[],
    );
    let built_catalog = catalog(&[built]);
    let credentials = provider_credentials(
        &built_catalog,
        "d1",
        None,
        &std::collections::BTreeMap::new(),
    )
    .expect("lookup")
    .expect("credentials");
    assert_eq!(credentials.get("model"), Some(&text("openai/gpt-4o")));
    assert_eq!(
        credentials.get("custom_llm_provider"),
        Some(&text("openai"))
    );
}

#[test]
fn a_prefixed_model_names_its_provider() {
    let prefixed = spec(
        "claude",
        "d2",
        &[("model", text("anthropic/claude-sonnet-5"))],
        &[],
    );
    let prefixed_catalog = catalog(&[prefixed]);
    let prefixed_credentials = provider_credentials(
        &prefixed_catalog,
        "d2",
        None,
        &std::collections::BTreeMap::new(),
    )
    .expect("lookup")
    .expect("credentials");
    assert_eq!(
        prefixed_credentials.get("custom_llm_provider"),
        Some(&text("anthropic"))
    );
}

#[test]
fn a_named_credential_is_resolved_and_dropped() {
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
fn the_credential_hash_ignores_non_credential_fields() {
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
fn the_credential_warning_names_the_missing_prefix() {
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
