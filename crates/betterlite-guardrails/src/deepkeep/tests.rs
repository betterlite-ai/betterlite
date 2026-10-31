#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::IntoFuture as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::{env, fs, io, process};

use super::error::DeepKeepError;
use super::{DeepKeepGuardrail, InputType, Inputs, RequestContext, Response};
use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::deepkeep_firewall::{Configured, UnreachableFallback};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tracing::subscriber::set_default;

fn guardrail() -> DeepKeepGuardrail {
    #[expect(
        clippy::disallowed_methods,
        reason = "unit tests need a local client for pure request construction"
    )]
    let client = reqwest::Client::builder().build().expect("client");
    DeepKeepGuardrail {
        settings: Configured {
            guardrail_name: "firewall".to_owned(),
            api_key: SecretString::from("key"),
            api_base: "https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api".to_owned(),
            deepkeep_firewall_id: "fixed".to_owned(),
            unreachable_fallback: UnreachableFallback::FailClosed,
            timeout: Duration::from_secs(600),
            extra_headers: BTreeMap::new(),
        },
        client,
    }
}

fn inputs() -> Inputs {
    Inputs {
        texts: vec!["original".to_owned()],
        images: Some(vec![GuardrailValue::Text("image".to_owned())]),
        tools: Some(vec![object(&[("name", text("tool"))])]),
        tool_calls: Some(vec![object(&[("id", text("call"))])]),
        structured_messages: Some(vec![object(&[("role", text("user"))])]),
        model: Some("model".to_owned()),
    }
}

fn context() -> RequestContext {
    RequestContext {
        request_data: BTreeMap::new(),
        input_type: InputType::Request,
        litellm_call_id: None,
        litellm_trace_id: None,
        litellm_version: "1.105.1".to_owned(),
        dynamic_params: BTreeMap::new(),
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

#[test]
fn deepkeep_guardrail_extracts_metadata_with_token_fallback() {
    let data = BTreeMap::from([
        (
            "metadata".to_owned(),
            object(&[("user_api_key_token", text("token"))]),
        ),
        (
            "litellm_metadata".to_owned(),
            object(&[("user_api_key_alias", text("alias"))]),
        ),
    ]);
    let metadata = DeepKeepGuardrail::extract_user_api_key_metadata(&data);
    assert_eq!(
        metadata["user_api_key_hash"],
        GuardrailValue::Text("token".to_owned())
    );
    assert_eq!(
        metadata["user_api_key_alias"],
        GuardrailValue::Text("alias".to_owned())
    );
}

#[test]
fn deepkeep_guardrail_explicit_hash_wins_over_token() {
    let data = BTreeMap::from([(
        "litellm_metadata".to_owned(),
        object(&[
            ("user_api_key_token", text("token")),
            ("user_api_key_hash", text("hash")),
        ]),
    )]);
    let metadata = DeepKeepGuardrail::extract_user_api_key_metadata(&data);
    assert_eq!(
        metadata["user_api_key_hash"],
        GuardrailValue::Text("hash".to_owned())
    );
}

#[test]
fn deepkeep_guardrail_metadata_prefers_litellm_and_filters_nulls() {
    let data = serde_json::from_str(
        r#"{
        "metadata": {"user_api_key_hash":"old", "user_api_key_alias":"old"},
        "litellm_metadata": {
            "user_api_key_hash":null, "user_api_key_token":"token",
            "user_api_key_alias":"alias", "user_api_key_user_id":"user",
            "user_api_key_user_email":"email", "user_api_key_team_id":"team",
            "user_api_key_team_alias":"team-alias", "user_api_key_end_user_id":"end-user",
            "user_api_key_org_id":"org", "unrelated":"ignored"
        }
    }"#,
    )
    .expect("metadata");
    let expected = serde_json::from_str(
        r#"{
        "user_api_key_hash":"token", "user_api_key_alias":"alias",
        "user_api_key_user_id":"user", "user_api_key_user_email":"email",
        "user_api_key_team_id":"team", "user_api_key_team_alias":"team-alias",
        "user_api_key_end_user_id":"end-user", "user_api_key_org_id":"org"
    }"#,
    )
    .expect("extracted metadata");
    assert_eq!(
        DeepKeepGuardrail::extract_user_api_key_metadata(&data),
        expected
    );
}

#[test]
fn deepkeep_guardrail_request_keeps_fixed_firewall_id() {
    let request = guardrail().build_request(
        inputs(),
        RequestContext {
            request_data: BTreeMap::new(),
            input_type: InputType::Request,
            litellm_call_id: Some("call".to_owned()),
            litellm_trace_id: Some("trace".to_owned()),
            litellm_version: "version".to_owned(),
            dynamic_params: BTreeMap::from([
                ("firewall_id".to_owned(), text("attacker")),
                ("custom".to_owned(), GuardrailValue::Boolean(true)),
            ]),
        },
    );
    assert_eq!(
        request.provider_params["firewall_id"],
        GuardrailValue::Text("fixed".to_owned())
    );
    assert_eq!(
        request.provider_params["custom"],
        GuardrailValue::Boolean(true)
    );
    assert_eq!(request.litellm_version, "version");
}

#[test]
fn deepkeep_guardrail_supports_pre_post_and_during_call_hooks() {
    assert_eq!(
        super::SUPPORTED_EVENT_HOOKS,
        [
            super::CallHook::Pre,
            super::CallHook::Post,
            super::CallHook::During
        ]
    );
}

#[test]
fn deepkeep_guardrail_empty_replacements_are_preserved() {
    let returned = Response {
        action: Some("NONE".to_owned()),
        texts: Some(Vec::new()),
        images: Some(Vec::new()),
        tools: Some(Vec::new()),
        tool_calls: Some(Vec::new()),
        structured_messages: Some(Vec::new()),
        blocked_reason: None,
    };
    let output = DeepKeepGuardrail::build_return_inputs(&inputs(), returned).expect("inputs");
    assert!(output.texts.is_empty());
    assert!(output.images.expect("images").is_empty());
    assert!(output.tools.expect("tools").is_empty());
    assert!(output.tool_calls.expect("tool_calls").is_empty());
    assert!(output.structured_messages.expect("messages").is_empty());
}

#[test]
fn deepkeep_guardrail_headers_include_secrets_without_debug_disclosure() {
    let mut deepkeep = guardrail();
    deepkeep.settings.api_key = SecretString::from("private-api-key");
    deepkeep.settings.extra_headers.insert(
        "Authorization".to_owned(),
        SecretString::from("Bearer private-extra-header"),
    );
    let headers = deepkeep.request_headers().expect("headers");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["x-api-key"], "private-api-key");
    assert_eq!(headers["authorization"], "Bearer private-extra-header");
    assert!(!format!("{headers:?}").contains("private"));
    deepkeep.settings.extra_headers.insert(
        "X-API-Key".to_owned(),
        SecretString::from("private-override"),
    );
    let overridden = deepkeep.request_headers().expect("headers");
    assert_eq!(overridden["x-api-key"], "private-override");
    assert!(!format!("{overridden:?}").contains("private"));
}

#[test]
fn deepkeep_guardrail_loads_from_config() {
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      unreachable_fallback: fail_open\n";
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    let settings = config
        .sections
        .deepkeep_firewall
        .guardrails
        .first()
        .expect("entry")
        .clone();
    assert_eq!(settings.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(
        settings.api_base,
        "https://deepkeep.test/v3/openai/beta/litellm_basic_guardrail_api"
    );
}

#[test]
fn deepkeep_guardrail_config_default_timeout_matches_http_handler() {
    assert_eq!(guardrail().settings.timeout, Duration::from_secs(600));
}

#[test]
fn deepkeep_guardrail_blocked_action_has_default_reason() {
    let returned = Response {
        action: Some("BLOCKED".to_owned()),
        texts: None,
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        blocked_reason: None,
    };
    let error = DeepKeepGuardrail::build_return_inputs(&inputs(), returned).expect_err("blocked");
    assert!(error.to_string().contains("Content violates policy"));
}

#[rstest]
#[case::timeout_fails_open(true, UnreachableFallback::FailOpen, true)]
#[case::timeout_fails_closed(true, UnreachableFallback::FailClosed, false)]
#[case::connection_failure_fails_open(false, UnreachableFallback::FailOpen, true)]
#[case::connection_failure_fails_closed(false, UnreachableFallback::FailClosed, false)]
#[tokio::test]
async fn deepkeep_guardrail_apply_honors_fail_open_when_unreachable(
    #[case] timeout: bool,
    #[case] unreachable_fallback: UnreachableFallback,
    #[case] expect_pass_through: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    if !timeout {
        // connection refused once nothing is listening on the port
        drop(listener);
    }
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = format!("{base}/v3/openai/beta/litellm_basic_guardrail_api");
    deepkeep.settings.unreachable_fallback = unreachable_fallback;
    let source = "guardrails:\n  - guardrail_name: firewall\n    litellm_params:\n      guardrail: deepkeep\n      api_key: key\n      api_base: https://deepkeep.test\n      deepkeep_firewall_id: fixed\n      timeout: '0.1'\n";
    let configured = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .expect("config");
    deepkeep.settings.timeout = configured.sections.deepkeep_firewall.guardrails[0].timeout;
    let result = deepkeep.apply(inputs(), context()).await;
    if expect_pass_through {
        assert_eq!(result.expect("fail-open passthrough"), inputs());
    } else if timeout {
        assert!(matches!(result, Err(DeepKeepError::Timeout { .. })));
    } else {
        assert!(matches!(result, Err(DeepKeepError::Request { .. })));
    }
}

#[test]
fn deepkeep_guardrail_accepts_non_blocked_action_and_applies_replacement() {
    let returned = Response {
        action: Some("ALLOWED".to_owned()),
        texts: Some(vec!["replacement".to_owned()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        blocked_reason: None,
    };
    let output = DeepKeepGuardrail::build_return_inputs(&inputs(), returned).expect("not blocked");
    assert_eq!(output.texts, vec!["replacement".to_owned()]);
}

#[test]
fn deepkeep_guardrail_inputs_default_texts_for_image_only_input() {
    let inputs: Inputs =
        serde_json::from_str(r#"{"images":["image"],"model":null}"#).expect("inputs");
    assert_eq!(inputs.texts, Vec::<String>::new());
    assert_eq!(
        inputs.images,
        Some(vec![GuardrailValue::Text("image".to_owned())])
    );
}

#[tokio::test]
async fn deepkeep_guardrail_fail_open_never_bypasses_request_build_errors() {
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base =
        "http://127.0.0.1:99999/v3/openai/beta/litellm_basic_guardrail_api".to_owned();
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    let result = deepkeep.apply(inputs(), context()).await;
    assert!(matches!(result, Err(DeepKeepError::RequestBuild { .. })));
}

#[tokio::test]
async fn deepkeep_guardrail_fail_open_logs_a_warning_with_guardrail_identity() {
    let log_path = env::temp_dir().join(format!("betterlite-deepkeep-fail-open-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = format!("{base}/v3/openai/beta/litellm_basic_guardrail_api");
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    deepkeep.settings.timeout = Duration::from_millis(100);
    let guard = set_default(subscriber);
    let result = deepkeep.apply(inputs(), context()).await;
    drop(guard);
    assert_eq!(result.expect("fail-open passthrough"), inputs());
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("firewall"));
    assert!(written.contains("fail_open"));
    assert!(!written.contains("key"));
}

#[tokio::test]
async fn deepkeep_guardrail_fail_open_never_logs_upstream_response_body() {
    let log_path = env::temp_dir().join(format!(
        "betterlite-deepkeep-fail-open-body-{}",
        process::id()
    ));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let (base, mut server) = serve(
        StatusCode::BAD_GATEWAY,
        "upstream failure echoing api_key=super-secret-upstream-token and \
         header Authorization: Bearer super-secret-extra-header"
            .as_bytes()
            .to_vec(),
    )
    .await;
    let mut deepkeep = guardrail();
    deepkeep.settings.api_key = SecretString::from("super-secret-upstream-token");
    deepkeep.settings.extra_headers = BTreeMap::from([(
        "X-API-Key".to_owned(),
        SecretString::from("super-secret-extra-header"),
    )]);
    deepkeep.settings.api_base = base;
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    deepkeep.settings.timeout = Duration::from_secs(5);
    let guard = set_default(subscriber);
    let result = deepkeep.apply(inputs(), context()).await;
    drop(guard);
    assert_eq!(result.expect("fail-open passthrough"), inputs());
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("fail_open"));
    assert!(written.contains("502"));
    assert!(!written.contains("super-secret-upstream-token"));
    assert!(!written.contains("super-secret-extra-header"));
    assert!(!written.contains("Authorization"));
    server.shutdown().await;
}

#[test]
fn deepkeep_guardrail_replays_the_deepkeep_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/deepkeep_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        let response = serde_json::from_slice::<Response>(&bytes);
        let inputs = serde_json::from_slice::<Inputs>(&bytes);
        response.is_ok() || inputs.is_ok()
    }));
}

#[rstest]
#[case::allowed(StatusCode::OK, "{}", "pass")]
#[case::invalid_json(StatusCode::OK, "not json", "decode")]
#[case::invalid_shape(StatusCode::OK, r#"{"texts":42}"#, "decode")]
#[case::bad_request(StatusCode::BAD_REQUEST, "bad request", "api")]
#[case::unauthorized(StatusCode::UNAUTHORIZED, "unauthorized", "api")]
#[case::server_error(StatusCode::INTERNAL_SERVER_ERROR, "error", "api")]
#[case::bad_gateway(StatusCode::BAD_GATEWAY, "unreachable", "pass")]
#[case::unavailable(StatusCode::SERVICE_UNAVAILABLE, "unreachable", "pass")]
#[case::gateway_timeout(StatusCode::GATEWAY_TIMEOUT, "unreachable", "pass")]
#[case::blocked(StatusCode::OK, r#"{"action":"BLOCKED"}"#, "blocked")]
#[tokio::test]
async fn deepkeep_guardrail_fail_open_requires_unreachable_service(
    #[case] status: StatusCode,
    #[case] body: &'static str,
    #[case] expected: &str,
) {
    let (base, mut server) = serve(status, body.as_bytes().to_vec()).await;
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = base;
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    let actual = match deepkeep.apply(inputs(), context()).await {
        Ok(output) => {
            assert_eq!(output, inputs());
            "pass"
        }
        Err(DeepKeepError::ResponseJson { .. }) => "decode",
        Err(DeepKeepError::Api {
            status: code,
            body: details,
        }) => {
            assert_eq!(code, status);
            assert_eq!(details, body);
            "api"
        }
        Err(DeepKeepError::Blocked { .. }) => "blocked",
        Err(error) => panic!("unexpected error: {error}"),
    };
    assert_eq!(actual, expected);
    server.shutdown().await;
}

#[rstest]
#[case::at_limit(StatusCode::OK, 0x0010_0000, true)]
#[case::over_limit(StatusCode::OK, 0x0010_0001, false)]
#[case::oversized_unreachable(StatusCode::BAD_GATEWAY, 0x0010_0001, true)]
#[case::oversized_unauthorized(StatusCode::UNAUTHORIZED, 0x0010_0001, false)]
#[tokio::test]
async fn deepkeep_guardrail_bounds_upstream_responses(
    #[case] status: StatusCode,
    #[case] size: usize,
    #[case] accepted: bool,
) {
    let body = format!("{{}}{}", " ".repeat(size.checked_sub(2).unwrap()));
    let (base, mut server) = serve(status, body.into_bytes()).await;
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = base;
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    let result = deepkeep.apply(inputs(), context()).await;
    assert_eq!(result.is_ok(), accepted);
    if status == StatusCode::UNAUTHORIZED {
        assert!(matches!(
            result,
            Err(DeepKeepError::Api {
                status: StatusCode::UNAUTHORIZED,
                ..
            })
        ));
    }
    server.shutdown().await;
}

#[rstest]
#[case::api_key(StatusCode::UNAUTHORIZED, "invalid secret-api-token", false)]
#[case::extra_header(
    StatusCode::UNAUTHORIZED,
    "Authorization: Bearer secret-header-token",
    false
)]
#[case::invalid_json(StatusCode::OK, r#""secret-api-token""#, false)]
#[case::blocked_reason(
    StatusCode::OK,
    r#"{"action":"BLOCKED","blocked_reason":"secret-api-token"}"#,
    true
)]
#[tokio::test]
async fn deepkeep_guardrail_errors_redact_echoed_credentials(
    #[case] status: StatusCode,
    #[case] body: &str,
    #[case] blocked: bool,
) {
    let (base, mut server) = serve(status, body.as_bytes().to_vec()).await;
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = base;
    deepkeep.settings.api_key = SecretString::from("secret-api-token");
    deepkeep.settings.extra_headers.insert(
        "Authorization".to_owned(),
        SecretString::from("Bearer secret-header-token"),
    );
    let error = deepkeep
        .apply(inputs(), context())
        .await
        .expect_err("unauthorized");
    if blocked {
        assert!(matches!(error, DeepKeepError::Blocked { .. }));
    } else if status.is_success() {
        assert!(matches!(error, DeepKeepError::ResponseJson { .. }));
        assert!(error.source().is_some());
    } else {
        assert!(matches!(
            error,
            DeepKeepError::Api {
                status: StatusCode::UNAUTHORIZED,
                ..
            }
        ));
    }
    assert!(!format!("{error} {error:?}").contains("secret-"));
    server.shutdown().await;
}

#[rstest]
#[case::unauthorized(StatusCode::UNAUTHORIZED, false)]
#[case::unreachable(StatusCode::BAD_GATEWAY, true)]
#[tokio::test]
async fn deepkeep_guardrail_rejects_non_utf8_error_details(
    #[case] status: StatusCode,
    #[case] fail_open: bool,
) {
    let (base, mut server) = serve(status, vec![0xff]).await;
    let mut deepkeep = guardrail();
    deepkeep.settings.api_base = base;
    deepkeep.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    let result = deepkeep.apply(inputs(), context()).await;
    if fail_open {
        assert_eq!(result.expect("unreachable passthrough"), inputs());
    } else {
        let error = result.expect_err("unauthorized");
        assert!(error.source().is_some());
        assert!(error.to_string().contains("401"));
        assert!(error.to_string().contains("UTF-8"));
    }
    server.shutdown().await;
}

async fn serve(status: StatusCode, body: Vec<u8>) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let router = Router::new().route(
        "/",
        post(move || {
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}
