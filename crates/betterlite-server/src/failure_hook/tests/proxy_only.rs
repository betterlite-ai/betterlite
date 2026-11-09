use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;

use super::super::hook::{FailureHook, FailureHookInput, FailureKey};
use super::super::ports::FailureLogging;
use super::super::proxy_only::{handle_proxy_only_error, is_proxy_only_error};
use super::support::{
    AlertParts, FakeCatalog, FakeCounter, FakeFactory, FakeLogging, FakeRoutes, FakeServiceLogging,
    FakeSlack, FakeUsageCache, HookParts, Journal, auth, deployment, details, drain, failure_hook,
    http_error, json, object, other_error, proxy_alerts, redactor,
};
use crate::guardrails::callback_caps::errors::{
    ProxyException, ProxyHookError, SensitiveRouteError,
};

#[derive(Clone, Copy)]
enum ErrorKind {
    Http(u16),
    Proxy(u16),
    Guardrail,
    Sensitive,
    Other,
}

fn proxy_error(code: &str) -> ProxyHookError {
    ProxyHookError::Proxy(Box::new(ProxyException {
        message: "over budget".to_owned(),
        error_type: "budget_exceeded".to_owned(),
        param: None,
        code: code.to_owned(),
        headers: BTreeMap::new(),
    }))
}

fn error(kind: ErrorKind) -> ProxyHookError {
    match kind {
        ErrorKind::Http(status) => http_error(status, "boom"),
        ErrorKind::Proxy(status) => proxy_error(&status.to_string()),
        ErrorKind::Guardrail => ProxyHookError::GuardrailRaised {
            message: "blocked".to_owned(),
            guardrail_name: Some("pii".to_owned()),
            status: 400,
            blocked_content: true,
        },
        ErrorKind::Sensitive => ProxyHookError::SensitiveDataRoute(Box::new(SensitiveRouteError {
            session_id: "s".to_owned(),
            route_to_model: "m".to_owned(),
            guardrail_name: None,
            sticky_session_routing: false,
            detection_info: BTreeMap::new(),
        })),
        ErrorKind::Other => other_error("plain"),
    }
}

#[rstest]
#[case::llm_route_http(ErrorKind::Http(500), Some("/v1/chat/completions"), None, true)]
#[case::info_route_proxy(ErrorKind::Proxy(400), Some("/v1/models"), None, true)]
#[case::guardrail(ErrorKind::Guardrail, Some("/v1/chat/completions"), None, true)]
#[case::management_route(ErrorKind::Http(500), Some("/key/generate"), None, false)]
#[case::missing_route(ErrorKind::Http(500), None, None, false)]
#[case::plain_error(ErrorKind::Other, Some("/v1/chat/completions"), None, false)]
#[case::auth_error_type(
    ErrorKind::Other,
    Some("/v1/chat/completions"),
    Some(ProxyErrorTypes::AuthError),
    true
)]
#[case::reroute(ErrorKind::Sensitive, Some("/v1/chat/completions"), None, false)]
fn proxy_only_errors_need_a_route_and_a_proxy_exception(
    #[case] kind: ErrorKind,
    #[case] route: Option<&str>,
    #[case] error_type: Option<ProxyErrorTypes>,
    #[case] expected: bool,
) {
    assert_eq!(
        is_proxy_only_error(&error(kind), error_type, route),
        expected
    );
}

fn hook_for(
    journal: &Journal,
    routes: FakeRoutes,
    catalog: FakeCatalog,
    log_client_errors: bool,
) -> (FailureHook, Arc<FakeFactory>) {
    let mut parts = AlertParts::new(None, Vec::new());
    parts.redactor = Some(redactor());
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(journal)),
        Arc::new(FakeSlack::new(journal, true)),
        Arc::new(FakeServiceLogging::default()),
    );
    let factory = Arc::new(FakeFactory::new(journal));
    let hook = failure_hook(HookParts {
        alerts,
        factory: Arc::clone(&factory),
        counter: Arc::new(FakeCounter::new(journal)),
        catalog: Arc::new(catalog),
        routes: Arc::new(routes),
        log_client_error_tracebacks: log_client_errors,
        new_call_id: "call-1".to_owned(),
    });
    (hook, factory)
}

async fn run_proxy_only(
    hook: &FailureHook,
    mut request_data: JsonMap,
    error: &ProxyHookError,
    logging: Option<Arc<dyn FailureLogging>>,
    route: &str,
) -> JsonMap {
    let auth = auth();
    let key = FailureKey {
        request_route: Some(route.to_owned()),
        team_id: Some("team".to_owned()),
        logged_key_metadata: object(&[("user_api_key_team_id", json("\"team\""))]),
    };
    let mut input = FailureHookInput {
        request_data: &mut request_data,
        error,
        auth: &auth,
        key,
        error_type: None,
        route: Some(route),
        traceback: None,
        logging,
        callbacks: &[],
    };
    handle_proxy_only_error(hook, &mut input).await;
    request_data
}

fn chat_routes() -> FakeRoutes {
    FakeRoutes::default().with(
        "/v1/chat/completions",
        vec![CallType::Acompletion, CallType::Completion],
    )
}

#[tokio::test]
async fn a_created_logging_object_gets_the_identity_input_and_key_metadata() {
    let journal = Journal::default();
    let catalog =
        FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]);
    let (hook, factory) = hook_for(&journal, chat_routes(), catalog, false);
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    let request_data = object(&[("model", json("\"gpt-4\"")), ("messages", messages.clone())]);
    let error = http_error(500, "boom");
    let lifted = run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    let created = factory.created();
    assert_eq!(created.len(), 1);
    let logging = &created[0];
    let environment = logging.environment().expect("environment");
    assert_eq!(environment.model, "gpt-4");
    assert_eq!(environment.user, "");
    assert_eq!(environment.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(environment.optional_params.get("messages"), Some(&messages));
    assert!(environment.litellm_params.get("model").is_none());
    assert!(
        environment
            .litellm_params
            .contains_key("proxy_rejected_before_routing")
    );
    let details = logging.details();
    assert_eq!(details.call_type, Some("acompletion".to_owned()));
    assert_eq!(details.messages, messages.as_array().cloned());
    assert!(details.no_upstream_llm_call);
    assert_eq!(lifted.get("litellm_call_id"), Some(&json("\"call-1\"")));
    let metadata = lifted
        .get("metadata")
        .and_then(|value| value.as_object())
        .expect("metadata");
    assert_eq!(
        metadata.get("user_api_key_team_id"),
        Some(&json("\"team\""))
    );
    assert_logging_order(&journal);
}

fn assert_logging_order(journal: &Journal) {
    let entries = journal.entries();
    assert!(entries[0].starts_with("create:/v1/chat/completions:call-1:"));
    assert_eq!(entries[1], "model_list:gpt-4:Some(\"team\")");
    assert_eq!(entries[2], "environment:gpt-4::Some(\"openai\")");
    assert_eq!(entries[3], "input:messages");
    assert_eq!(entries[4], "call_type:Acompletion");
    assert_eq!(entries[5], "no_upstream_call");
    assert_eq!(entries[6], "pre_call:messages:");
    assert_eq!(entries[7], "async_failure:true");
    assert!(entries.contains(&"sync_failure:true".to_owned()));
}

#[tokio::test]
async fn request_fields_split_into_litellm_and_optional_params() {
    let journal = Journal::default();
    let (hook, factory) = hook_for(&journal, chat_routes(), FakeCatalog::new(&journal), false);
    let error = http_error(500, "boom");
    let request_data = object(&[
        ("model", json("\"gpt-4\"")),
        ("user", json("\"u1\"")),
        ("litellm_logging_obj", json("{}")),
        ("metadata", json(r#"{"m":1}"#)),
        ("litellm_call_id", json("\"call-7\"")),
        ("no_log", json("true")),
        ("cooldown_time", json("30")),
        ("temperature", json("0.2")),
    ]);
    run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    let created = factory.created();
    let environment = created[0].environment().expect("environment");
    let litellm_keys: BTreeSet<&str> = environment
        .litellm_params
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        litellm_keys,
        BTreeSet::from([
            "cooldown_time",
            "litellm_call_id",
            "metadata",
            "no_log",
            "proxy_rejected_before_routing",
        ])
    );
    let optional_keys: BTreeSet<&str> = environment
        .optional_params
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(optional_keys, BTreeSet::from(["temperature"]));
}

#[tokio::test]
async fn an_existing_logging_object_is_reused_without_merging_key_metadata() {
    let journal = Journal::default();
    let (hook, factory) = hook_for(&journal, chat_routes(), FakeCatalog::new(&journal), false);
    let existing = Arc::new(FakeLogging::new(&journal, false, None));
    let logging: Arc<dyn FailureLogging> = existing;
    let error = http_error(500, "boom");
    let request_data = object(&[
        ("model", json("\"gpt-4\"")),
        ("messages", json(r#"[{"role":"user"}]"#)),
    ]);
    let lifted = run_proxy_only(
        &hook,
        request_data,
        &error,
        Some(logging),
        "/v1/chat/completions",
    )
    .await;
    drain(&hook.alerts.tasks).await;
    assert!(factory.created().is_empty());
    assert!(lifted.get("metadata").is_none());
    let entries = journal.entries();
    assert!(entries.contains(&"call_type:Acompletion".to_owned()));
    assert!(entries.contains(&"pre_call:messages:".to_owned()));
}

#[tokio::test]
async fn a_cache_hit_counts_as_reaching_a_deployment() {
    let journal = Journal::default();
    let catalog =
        FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]);
    let (hook, factory) = hook_for(&journal, chat_routes(), catalog, false);
    let existing = Arc::new(FakeLogging::new(&journal, false, None));
    let mut logging_details = details();
    logging_details.cache_hit = true;
    existing.set_details(logging_details);
    let shared = Arc::clone(&existing);
    let logging: Arc<dyn FailureLogging> = shared;
    let error = http_error(500, "boom");
    let request_data = object(&[("model", json("\"gpt-4\""))]);
    run_proxy_only(
        &hook,
        request_data,
        &error,
        Some(logging),
        "/v1/chat/completions",
    )
    .await;
    drain(&hook.alerts.tasks).await;
    assert!(factory.created().is_empty());
    let environment = existing.environment().expect("environment");
    assert!(
        !environment
            .litellm_params
            .contains_key("proxy_rejected_before_routing")
    );
    assert_eq!(
        environment.litellm_params.get("custom_llm_provider"),
        Some(&json("\"openai\""))
    );
}

#[tokio::test]
async fn pass_through_calls_stop_before_the_upstream_marker_and_pre_call() {
    let journal = Journal::default();
    let (hook, _factory) = hook_for(&journal, chat_routes(), FakeCatalog::new(&journal), false);
    let existing = Arc::new(
        FakeLogging::new(&journal, false, None).with_call_type(CallType::PassThroughEndpoint),
    );
    let logging: Arc<dyn FailureLogging> = existing;
    let error = http_error(500, "boom");
    let request_data = object(&[
        ("model", json("\"gpt-4\"")),
        ("messages", json(r#"[{"role":"user"}]"#)),
    ]);
    run_proxy_only(
        &hook,
        request_data,
        &error,
        Some(logging),
        "/v1/chat/completions",
    )
    .await;
    drain(&hook.alerts.tasks).await;
    let entries = journal.entries();
    assert!(entries.contains(&"input:messages".to_owned()));
    assert!(!entries.iter().any(|entry| entry.starts_with("call_type:")));
    assert!(!entries.contains(&"no_upstream_call".to_owned()));
    assert!(!entries.iter().any(|entry| entry.starts_with("pre_call:")));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.starts_with("async_failure:"))
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.starts_with("sync_failure:"))
    );
}

#[tokio::test]
async fn the_routes_single_operation_call_type_wins_over_the_body_shape() {
    let journal = Journal::default();
    let routes = FakeRoutes::default().with(
        "/v1/chat/completions",
        vec![CallType::Embedding, CallType::Aembedding],
    );
    let (hook, _factory) = hook_for(&journal, routes, FakeCatalog::new(&journal), false);
    let error = http_error(500, "boom");
    let request_data = object(&[("messages", json(r#"[{"role":"user"}]"#))]);
    run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    assert!(
        journal
            .entries()
            .contains(&"call_type:Embedding".to_owned())
    );
}

#[tokio::test]
async fn a_mixed_route_falls_back_to_the_body_shape() {
    let journal = Journal::default();
    let routes = FakeRoutes::default().with(
        "/v1/chat/completions",
        vec![CallType::Acompletion, CallType::Embedding],
    );
    let (hook, _factory) = hook_for(&journal, routes, FakeCatalog::new(&journal), false);
    let error = http_error(500, "boom");
    let request_data = object(&[("messages", json(r#"[{"role":"user"}]"#))]);
    run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    assert!(
        journal
            .entries()
            .contains(&"call_type:Acompletion".to_owned())
    );
}

#[tokio::test]
async fn key_metadata_overwrites_existing_fields_and_keeps_the_rest() {
    let journal = Journal::default();
    let (hook, _factory) = hook_for(&journal, chat_routes(), FakeCatalog::new(&journal), false);
    let error = http_error(500, "boom");
    let request_data = object(&[(
        "metadata",
        json(r#"{"user_api_key_team_id":"old","keep":"yes"}"#),
    )]);
    let lifted = run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    let metadata = lifted
        .get("metadata")
        .and_then(|value| value.as_object())
        .expect("metadata");
    assert_eq!(
        metadata.get("user_api_key_team_id"),
        Some(&json("\"team\""))
    );
    assert_eq!(metadata.get("keep"), Some(&json("\"yes\"")));
}

#[derive(Clone, Copy)]
enum TracebackCase {
    ClientError,
    ClientErrorLogged,
    ServerError,
    GuardrailBlock,
    ProxyClientCode,
    NonNumericProxyCode,
    BelowClientRange,
}

#[rstest]
#[case::client_error_skips(TracebackCase::ClientError, false)]
#[case::client_error_logged(TracebackCase::ClientErrorLogged, true)]
#[case::server_error_keeps_it(TracebackCase::ServerError, true)]
#[case::guardrail_block_skips_it(TracebackCase::GuardrailBlock, false)]
#[case::proxy_client_code_skips_it(TracebackCase::ProxyClientCode, false)]
#[case::non_numeric_proxy_code_keeps_it(TracebackCase::NonNumericProxyCode, true)]
#[case::status_below_400_keeps_it(TracebackCase::BelowClientRange, true)]
#[tokio::test]
async fn tracebacks_follow_the_client_error_rule(
    #[case] case: TracebackCase,
    #[case] expected: bool,
) {
    let journal = Journal::default();
    let log_client_errors = matches!(case, TracebackCase::ClientErrorLogged);
    let (hook, _factory) = hook_for(
        &journal,
        chat_routes(),
        FakeCatalog::new(&journal),
        log_client_errors,
    );
    let error = match case {
        TracebackCase::ClientError | TracebackCase::ClientErrorLogged => {
            error(ErrorKind::Http(400))
        }
        TracebackCase::ServerError => error(ErrorKind::Http(500)),
        TracebackCase::GuardrailBlock => error(ErrorKind::Guardrail),
        TracebackCase::ProxyClientCode => error(ErrorKind::Proxy(429)),
        TracebackCase::NonNumericProxyCode => proxy_error("not-a-number"),
        TracebackCase::BelowClientRange => error(ErrorKind::Http(302)),
    };
    let request_data = object(&[("messages", json(r#"[{"role":"user"}]"#))]);
    run_proxy_only(&hook, request_data, &error, None, "/v1/chat/completions").await;
    drain(&hook.alerts.tasks).await;
    let entries = journal.entries();
    assert!(entries.contains(&format!("async_failure:{expected}")));
    assert!(entries.contains(&format!("sync_failure:{expected}")));
}
