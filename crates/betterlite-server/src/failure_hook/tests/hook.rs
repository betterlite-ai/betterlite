use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::routes::CallType;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rstest::rstest;

use super::super::alerts::ProxyAlerts;
use super::super::hook::{FailureHook, FailureHookInput, FailureKey};
use super::super::ports::{AlertType, AlertingClient, FailureLogging};
use super::support::{
    AlertParts, FakeCallback, FakeCatalog, FakeCounter, FakeFactory, FakeLogging, FakeRoutes,
    FakeServiceLogging, FakeSlack, FakeUsageCache, HookParts, Journal, auth, deployment, details,
    drain, failure_hook, http_error, json, object, other_error, proxy_alerts, redactor, replace,
};
use crate::guardrails::callback_caps::errors::{HttpException, ProxyException, ProxyHookError};
use crate::guardrails::callback_caps::traits::{FailureReply, ProxyCallback};

struct Harness {
    hook: FailureHook,
    alerts: Arc<ProxyAlerts>,
    slack: Arc<FakeSlack>,
    factory: Arc<FakeFactory>,
    journal: Journal,
}

fn harness(alerting: Option<Vec<AlertingClient>>, alert_types: Vec<AlertType>) -> Harness {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let mut parts = AlertParts::new(alerting, alert_types);
    parts.redactor = Some(redactor());
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    let factory = Arc::new(FakeFactory::new(&journal));
    let hook = failure_hook(HookParts {
        alerts: Arc::clone(&alerts),
        factory: Arc::clone(&factory),
        counter: Arc::new(FakeCounter::new(&journal)),
        catalog: Arc::new(
            FakeCatalog::new(&journal).with("gpt-4", vec![deployment(Some("openai"), "gpt-4")]),
        ),
        routes: Arc::new(FakeRoutes::default().with(
            "/v1/chat/completions",
            vec![CallType::Acompletion, CallType::Completion],
        )),
        log_client_error_tracebacks: false,
        new_call_id: "call-1".to_owned(),
    });
    Harness {
        hook,
        alerts,
        slack,
        factory,
        journal,
    }
}

fn key(route: &str) -> FailureKey {
    FailureKey {
        request_route: Some(route.to_owned()),
        team_id: Some("team".to_owned()),
        logged_key_metadata: object(&[]),
    }
}

fn four_callbacks(journal: &Journal) -> Vec<Arc<dyn ProxyCallback>> {
    vec![
        FakeCallback::new(journal, "one", Ok(replace(401, "first"))),
        FakeCallback::new(journal, "two", Err(http_error(403, "raised"))),
        FakeCallback::new(journal, "three", Err(other_error("ignored"))),
        FakeCallback::new(journal, "four", Ok(replace(418, "later"))),
    ]
}

fn order_request_data() -> JsonMap {
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    object(&[
        ("model", json("\"gpt-4\"")),
        ("messages", messages),
        ("litellm_call_id", json("\"client-call\"")),
        ("litellm_logging_obj", json("{}")),
        ("standard_logging_object", json(r#"{"stale":true}"#)),
    ])
}

fn assert_hook_order(journal: &Journal) {
    let entries = journal.entries();
    assert_eq!(entries[0], "status:request_status:client-call:fail:400");
    assert!(
        entries[1].starts_with("create:/v1/chat/completions:client-call:"),
        "{entries:?}"
    );
    let seen: Vec<&String> = entries
        .iter()
        .filter(|entry| entry.starts_with("callback:"))
        .collect();
    assert_eq!(seen.len(), 4);
    assert!(seen[0].starts_with("callback:one:"));
    assert!(seen[1].starts_with("callback:two:"));
    assert!(seen[3].starts_with("callback:four:"));
    assert!(!seen[0].contains("litellm_logging_obj"), "{}", seen[0]);
}

#[tokio::test]
async fn the_hook_runs_litellm_steps_in_order_and_returns_the_first_http_error() {
    let harness = harness(Some(vec![AlertingClient::Slack]), Vec::new());
    let auth = auth();
    let callbacks = four_callbacks(&harness.journal);
    let error = http_error(500, "boom");
    let mut request_data = order_request_data();
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/v1/chat/completions"),
        error_type: None,
        route: Some("/v1/chat/completions"),
        traceback: Some("tb"),
        logging: None,
        callbacks: &callbacks,
    };
    let transformed = harness.hook.post_call_failure_hook(input).await;
    drain(&harness.alerts.tasks).await;
    assert_eq!(
        transformed,
        Some(HttpException {
            status: 401,
            detail: "first".to_owned()
        })
    );
    assert!(request_data.get("litellm_logging_obj").is_none());
    assert!(request_data.get("standard_logging_object").is_none());
    let created = harness.factory.created();
    let environment = created[0].environment().expect("environment");
    assert!(
        environment
            .optional_params
            .get("standard_logging_object")
            .is_none()
    );
    assert_hook_order(&harness.journal);
}

#[tokio::test]
async fn a_raised_http_error_is_used_when_no_callback_returned_one() {
    let harness = harness(None, Vec::new());
    let auth = auth();
    let callbacks: Vec<Arc<dyn ProxyCallback>> = vec![
        FakeCallback::new(&harness.journal, "first", Err(http_error(403, "raised"))),
        FakeCallback::new(&harness.journal, "second", Ok(replace(418, "later"))),
    ];
    let error = http_error(500, "boom");
    let mut request_data = object(&[("litellm_call_id", json("\"c\""))]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/v1/chat/completions"),
        error_type: None,
        route: Some("/v1/chat/completions"),
        traceback: None,
        logging: None,
        callbacks: &callbacks,
    };
    let transformed = harness.hook.post_call_failure_hook(input).await;
    assert_eq!(
        transformed,
        Some(HttpException {
            status: 403,
            detail: "raised".to_owned()
        })
    );
}

#[derive(Clone, Copy)]
enum AlertCase {
    ClientHttp,
    ClientProxy,
    ServerHttp,
    Guardrail,
}

fn alert_error(case: AlertCase) -> ProxyHookError {
    match case {
        AlertCase::ClientHttp => http_error(400, "bad request"),
        AlertCase::ClientProxy => ProxyHookError::Proxy(Box::new(ProxyException {
            message: "over budget".to_owned(),
            error_type: "budget_exceeded".to_owned(),
            param: None,
            code: "429".to_owned(),
            headers: BTreeMap::new(),
        })),
        AlertCase::ServerHttp => http_error(500, "boom"),
        AlertCase::Guardrail => ProxyHookError::GuardrailRaised {
            message: "blocked".to_owned(),
            guardrail_name: None,
            status: 400,
            blocked_content: true,
        },
    }
}

async fn run_hook(
    harness: &Harness,
    error: &ProxyHookError,
    request_data: &mut JsonMap,
    route: &str,
) {
    let auth = auth();
    let input = FailureHookInput {
        request_data,
        error,
        auth: &auth,
        key: key(route),
        error_type: None,
        route: Some(route),
        traceback: None,
        logging: None,
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
}

#[rstest]
#[case::client_http(AlertCase::ClientHttp, false)]
#[case::client_proxy(AlertCase::ClientProxy, false)]
#[case::server_http(AlertCase::ServerHttp, true)]
#[case::guardrail(AlertCase::Guardrail, true)]
#[tokio::test]
async fn llm_exception_alerts_skip_client_errors(#[case] case: AlertCase, #[case] expected: bool) {
    let harness = harness(
        Some(vec![AlertingClient::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let error = alert_error(case);
    let mut request_data = object(&[]);
    run_hook(&harness, &error, &mut request_data, "/v1/chat/completions").await;
    drain(&harness.alerts.tasks).await;
    let sent = harness.slack.sent();
    assert_eq!(sent.len(), usize::from(expected));
    if expected {
        assert_eq!(sent[0].message, format!("LLM API call failed: `{error}`"));
    }
}

#[tokio::test]
async fn the_llm_exception_alert_is_redacted() {
    let harness = harness(
        Some(vec![AlertingClient::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let error = http_error(500, "provider sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa broke");
    let mut request_data = object(&[]);
    run_hook(&harness, &error, &mut request_data, "/v1/chat/completions").await;
    drain(&harness.alerts.tasks).await;
    let sent = harness.slack.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].message.contains("REDACTED"));
    assert!(!sent[0].message.contains("sk-aaaa"));
}

#[tokio::test]
async fn the_traceback_is_redacted_before_callbacks_see_it() {
    let harness = harness(None, Vec::new());
    let auth = auth();
    let callbacks: Vec<Arc<dyn ProxyCallback>> = vec![FakeCallback::new(
        &harness.journal,
        "one",
        Ok(FailureReply::Original),
    )];
    let error = http_error(500, "boom");
    let mut request_data = object(&[]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/v1/chat/completions"),
        error_type: None,
        route: Some("/v1/chat/completions"),
        traceback: Some("failed sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        logging: None,
        callbacks: &callbacks,
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    drain(&harness.alerts.tasks).await;
    let entries = harness.journal.entries();
    let callback = entries
        .iter()
        .find(|entry| entry.starts_with("callback:one:"))
        .expect("callback ran");
    assert!(callback.contains("REDACTED"));
    assert!(!callback.contains("sk-aaaa"));
}

#[tokio::test]
async fn the_standard_logging_object_is_dropped_then_lifted_from_the_logging_object() {
    let harness = harness(None, Vec::new());
    let auth = auth();
    let logging = Arc::new(FakeLogging::new(&harness.journal, false, None));
    let mut logging_details = details();
    logging_details.standard_logging_object = Some(json(r#"{"lifted":true}"#));
    logging.set_details(logging_details);
    let handle: Arc<dyn FailureLogging> = logging;
    let error = http_error(500, "boom");
    let mut request_data = object(&[
        ("litellm_logging_obj", json("{}")),
        ("standard_logging_object", json(r#"{"stale":true}"#)),
    ]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/key/generate"),
        error_type: None,
        route: Some("/key/generate"),
        traceback: None,
        logging: Some(handle),
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    assert_eq!(
        request_data.get("standard_logging_object"),
        Some(&json(r#"{"lifted":true}"#))
    );
    assert!(request_data.get("litellm_logging_obj").is_none());
}

#[tokio::test]
async fn management_routes_do_not_create_a_logging_object() {
    let harness = harness(Some(vec![AlertingClient::Slack]), Vec::new());
    let error = http_error(500, "boom");
    let mut request_data = object(&[("litellm_call_id", json("\"client-call\""))]);
    run_hook(&harness, &error, &mut request_data, "/key/generate").await;
    drain(&harness.alerts.tasks).await;
    assert!(harness.factory.created().is_empty());
    let entries = harness.journal.entries();
    assert_eq!(entries[0], "status:request_status:client-call:fail:400");
    assert!(!entries.iter().any(|entry| entry.starts_with("create:")));
    assert!(!entries.iter().any(|entry| entry.starts_with("pre_call:")));
}

#[tokio::test]
async fn a_created_logging_object_lifts_its_call_type_onto_the_request() {
    let harness = harness(None, Vec::new());
    let error = http_error(500, "boom");
    let messages = json(r#"[{"role":"user","content":"hi"}]"#);
    let mut request_data = object(&[("model", json("\"gpt-4\"")), ("messages", messages)]);
    run_hook(&harness, &error, &mut request_data, "/v1/chat/completions").await;
    drain(&harness.alerts.tasks).await;
    assert_eq!(
        request_data.get("litellm_call_id"),
        Some(&json("\"call-1\""))
    );
    assert_eq!(
        request_data.get("call_type"),
        Some(&json("\"acompletion\""))
    );
    assert!(request_data.get("combined_usage_object").is_none());
}

#[tokio::test]
async fn an_auth_failure_on_an_info_route_lifts_the_route_as_call_type() {
    let harness = harness(None, Vec::new());
    let auth = auth();
    let error = other_error("invalid api key");
    let mut request_data = object(&[]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/v1/models"),
        error_type: Some(ProxyErrorTypes::AuthError),
        route: Some("/v1/models"),
        traceback: None,
        logging: None,
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    drain(&harness.alerts.tasks).await;
    assert_eq!(request_data.get("call_type"), Some(&json("\"/v1/models\"")));
}

#[tokio::test]
async fn an_existing_logging_object_with_a_handoff_estimates_input_usage() {
    let harness = harness(None, Vec::new());
    let auth = auth();
    let logging = Arc::new(FakeLogging::new(&harness.journal, false, None));
    let mut logging_details = details();
    logging_details.first_api_call_start_time = Some(json("\"2026-01-01T00:00:00Z\""));
    logging_details.call_type = Some("acompletion".to_owned());
    logging_details.model = Some("gpt-4".to_owned());
    logging_details.messages = Some(vec![json(r#"{"role":"user","content":"hi"}"#)]);
    logging.set_details(logging_details);
    let handle: Arc<dyn FailureLogging> = logging;
    let error = http_error(500, "boom");
    let mut request_data = object(&[]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/key/generate"),
        error_type: None,
        route: Some("/key/generate"),
        traceback: None,
        logging: Some(handle),
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    assert_eq!(
        request_data.get("combined_usage_object"),
        Some(&json(
            r#"{"completion_tokens":0,"prompt_tokens":11,"total_tokens":11}"#
        ))
    );
    assert_eq!(request_data.get("response_cost"), Some(&json("0")));
    assert_eq!(
        request_data.get("call_type"),
        Some(&json("\"acompletion\""))
    );
}

#[tokio::test]
async fn baseline_estimates_are_invalidated_before_the_request_status() {
    let harness = harness(Some(vec![AlertingClient::Slack]), Vec::new());
    let auth = auth();
    let logging = Arc::new(FakeLogging::new(&harness.journal, true, None));
    let handle: Arc<dyn FailureLogging> = logging;
    let error = http_error(500, "boom");
    let mut request_data = object(&[("litellm_call_id", json("\"c\""))]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/key/generate"),
        error_type: None,
        route: Some("/key/generate"),
        traceback: None,
        logging: Some(handle),
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    let entries = harness.journal.entries();
    assert_eq!(entries[0], "invalidate:failed_request:true");
    assert_eq!(entries[1], "status:request_status:c:fail:400");
}

#[tokio::test]
async fn a_logging_object_without_a_baseline_context_is_not_invalidated() {
    let harness = harness(Some(vec![AlertingClient::Slack]), Vec::new());
    let auth = auth();
    let logging = Arc::new(FakeLogging::new(&harness.journal, false, None));
    let handle: Arc<dyn FailureLogging> = logging;
    let error = http_error(500, "boom");
    let mut request_data = object(&[("litellm_call_id", json("\"c\""))]);
    let input = FailureHookInput {
        request_data: &mut request_data,
        error: &error,
        auth: &auth,
        key: key("/key/generate"),
        error_type: None,
        route: Some("/key/generate"),
        traceback: None,
        logging: Some(handle),
        callbacks: &[],
    };
    drop(harness.hook.post_call_failure_hook(input).await);
    let entries = harness.journal.entries();
    assert!(!entries.iter().any(|entry| entry.starts_with("invalidate:")));
}
