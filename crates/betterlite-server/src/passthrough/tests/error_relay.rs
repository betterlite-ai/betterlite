use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_http::error::HttpError;
use betterlite_http::passthrough::error_body::{
    ChunkStream, PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS,
};
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_models::decisions::Json;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::header::HeaderValue;
use http::{HeaderMap, StatusCode};

use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::passthrough::error_relay::{
    FailureReporting, LoggingRedaction, MessageRedaction, PassthroughFailureHook,
    PassthroughHookError, RelayResponse, log_upstream_failure, relay_reporting_failures,
    sanitize_upstream_error_body, truncate_upstream_error_body,
};
use crate::passthrough::request_context::PassthroughLoggingObj;

#[derive(Default)]
struct RecordingHook {
    calls: Mutex<Vec<(ProxyHookError, RequestData)>>,
}

#[async_trait]
impl PassthroughFailureHook for RecordingHook {
    async fn post_call_failure_hook(
        &self,
        _user_api_key_dict: &PrincipalFacts,
        original_exception: &ProxyHookError,
        request_data: &mut RequestData,
    ) -> Result<(), PassthroughHookError> {
        self.calls
            .lock()
            .unwrap()
            .push((original_exception.clone(), request_data.clone()));
        Ok(())
    }
}

struct BrokenHook;

#[async_trait]
impl PassthroughFailureHook for BrokenHook {
    async fn post_call_failure_hook(
        &self,
        _user_api_key_dict: &PrincipalFacts,
        _original_exception: &ProxyHookError,
        _request_data: &mut RequestData,
    ) -> Result<(), PassthroughHookError> {
        Err(PassthroughHookError)
    }
}

struct FixedRedaction(bool);

impl MessageRedaction for FixedRedaction {
    fn should_redact(&self, _model_call_details: &Fields) -> bool {
        self.0
    }
}

fn chunks(items: Vec<Result<Vec<u8>, HttpError>>) -> ChunkStream {
    stream::iter(items).boxed()
}

async fn body_bytes(stream: ChunkStream) -> Vec<Vec<u8>> {
    stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|chunk| chunk.unwrap())
        .collect()
}

fn outcomes(items: Vec<Result<Vec<u8>, HttpError>>) -> (Vec<Vec<u8>>, usize) {
    let mut delivered = Vec::new();
    let mut failed = 0;
    for item in items {
        match item {
            Ok(bytes) => delivered.push(bytes),
            Err(_) => failed = failed.saturating_add(1),
        }
    }
    (delivered, failed)
}

fn data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}

fn logging_obj(text: &str) -> PassthroughLoggingObj {
    PassthroughLoggingObj {
        model_call_details: data(text),
    }
}

fn response(status: StatusCode, headers: HeaderMap) -> RelayResponse {
    RelayResponse {
        status,
        method: "POST".to_owned(),
        url: "https://up.example/v1?token=secret#frag".to_owned(),
        headers,
        body: chunks(vec![Ok(b"bad \x01body\t".to_vec())]),
    }
}

async fn reported(hook: &dyn PassthroughFailureHook, redaction: &dyn MessageRedaction) -> Vec<u8> {
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let relayed = log_upstream_failure(
        response(StatusCode::INTERNAL_SERVER_ERROR, HeaderMap::new()),
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data(r#"{"model": "m"}"#),
            logging_obj: &details,
            hook,
            redaction,
        },
    )
    .await;
    assert_eq!(relayed.status, StatusCode::INTERNAL_SERVER_ERROR);
    body_bytes(relayed.body).await.into_iter().next().unwrap()
}

async fn assert_reported() {
    let hook = RecordingHook::default();
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("12"));
    headers.insert("x-up", HeaderValue::from_static("1"));
    let relayed = log_upstream_failure(
        response(StatusCode::INTERNAL_SERVER_ERROR, headers),
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data(r#"{"model": "m"}"#),
            logging_obj: &details,
            hook: &hook,
            redaction: &FixedRedaction(false),
        },
    )
    .await;
    assert_eq!(
        body_bytes(relayed.body).await,
        vec![b"bad \x01body\t".to_vec()]
    );
    // the relayed body is replayed, so its framing headers no longer describe it
    assert!(relayed.headers.get("content-length").is_none());
    assert_eq!(relayed.headers.get("x-up").unwrap(), "1");
    let calls = hook.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        ProxyHookError::Http {
            status: 500,
            detail: "Upstream passthrough request failed with status 500: bad body".to_owned(),
        }
    );
    assert_eq!(calls[0].1["model"], Json::Text("m".to_owned()));
}

async fn assert_below_400() {
    let hook = RecordingHook::default();
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("12"));
    let relayed = log_upstream_failure(
        response(StatusCode::OK, headers),
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data("{}"),
            logging_obj: &details,
            hook: &hook,
            redaction: &FixedRedaction(false),
        },
    )
    .await;
    assert!(hook.calls.lock().unwrap().is_empty());
    assert_eq!(
        body_bytes(relayed.body).await,
        vec![b"bad \x01body\t".to_vec()]
    );
    // a response under 400 is returned untouched, framing headers included
    assert_eq!(relayed.headers.get("content-length").unwrap(), "12");
}

async fn assert_broken_hook() {
    let body = reported(&BrokenHook, &FixedRedaction(false)).await;
    assert_eq!(body, b"bad \x01body\t".to_vec());
}

async fn assert_redacted() {
    let hook = RecordingHook::default();
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let relayed = log_upstream_failure(
        response(StatusCode::BAD_GATEWAY, HeaderMap::new()),
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data("{}"),
            logging_obj: &details,
            hook: &hook,
            redaction: &FixedRedaction(true),
        },
    )
    .await;
    assert_eq!(
        body_bytes(relayed.body).await,
        vec![b"bad \x01body\t".to_vec()]
    );
    assert_eq!(
        hook.calls.lock().unwrap()[0].0,
        ProxyHookError::Http {
            status: 502,
            detail: "Upstream passthrough request failed with status 502: redacted-by-litellm"
                .to_owned(),
        }
    );
}

fn assert_redaction_settings() {
    let global = LoggingRedaction { global: true };
    let empty = PassthroughLoggingObj::default();
    assert!(global.should_redact(&empty.model_call_details));
    let opted_out = logging_obj(
        r#"{"litellm_params": {"metadata": {"headers": {"litellm-disable-message-redaction": "true"}}}}"#,
    );
    assert!(!global.should_redact(&opted_out.model_call_details));
    let requested_off = logging_obj(
        r#"{"standard_callback_dynamic_params": {"turn_off_message_logging": "false"}}"#,
    );
    assert!(!global.should_redact(&requested_off.model_call_details));
    let enabled = logging_obj(
        r#"{"litellm_params": {"litellm_metadata": {"headers": {"x-litellm-enable-message-redaction": "true"}}}}"#,
    );
    assert!(LoggingRedaction { global: false }.should_redact(&enabled.model_call_details));
}

async fn assert_stream_error() {
    let hook = RecordingHook::default();
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let relayed: Vec<_> = relay_reporting_failures(
        chunks(vec![
            Ok(b"a".to_vec()),
            Err(HttpError::Timeout),
            Ok(b"never".to_vec()),
        ]),
        200,
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data("{}"),
            logging_obj: &details,
            hook: &hook,
            redaction: &FixedRedaction(false),
        },
    )
    .collect()
    .await;
    assert_eq!(outcomes(relayed), (vec![b"a".to_vec()], 1));
    let calls = hook.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        ProxyHookError::Other("the request timed out".to_owned())
    );
}

async fn assert_no_double_report() {
    let hook = RecordingHook::default();
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let relayed: Vec<_> = relay_reporting_failures(
        chunks(vec![Err(HttpError::Timeout)]),
        500,
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data("{}"),
            logging_obj: &details,
            hook: &hook,
            redaction: &FixedRedaction(false),
        },
    )
    .collect()
    .await;
    assert_eq!(outcomes(relayed), (Vec::new(), 1));
    assert!(hook.calls.lock().unwrap().is_empty());
}

async fn assert_broken_stream_hook() {
    let details = PassthroughLoggingObj::default();
    let principal = PrincipalFacts::default();
    let relayed: Vec<_> = relay_reporting_failures(
        chunks(vec![Err(HttpError::Timeout)]),
        200,
        FailureReporting {
            user_api_key_dict: &principal,
            request_payload: data("{}"),
            logging_obj: &details,
            hook: &BrokenHook,
            redaction: &FixedRedaction(false),
        },
    )
    .collect()
    .await;
    assert_eq!(outcomes(relayed), (Vec::new(), 1));
}

#[test]
fn passthrough_error_body_truncate() {
    let short = "short error";
    assert_eq!(truncate_upstream_error_body(short), short);

    let exact = "x".repeat(PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS);
    assert_eq!(truncate_upstream_error_body(&exact), exact);

    let long = "x".repeat(PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS + 1);
    assert_eq!(
        truncate_upstream_error_body(&long),
        format!(
            "{}... (truncated at {PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS} chars)",
            "x".repeat(PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS)
        )
    );

    // Python counts characters, not bytes
    let wide = "\u{e9}".repeat(PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS + 1);
    let truncated = truncate_upstream_error_body(&wide);
    let suffix = "... (truncated at 4096 chars)".chars().count();
    assert_eq!(
        truncated.chars().count(),
        PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS.saturating_add(suffix)
    );
}

#[test]
fn passthrough_error_body_sanitize() {
    assert_eq!(
        sanitize_upstream_error_body("  a \n\t b \u{7f}c  "),
        "a b c"
    );
    assert_eq!(
        sanitize_upstream_error_body("caf\u{e9} \u{20ac}"),
        "caf\u{e9} \u{20ac}"
    );
    // a no-break space and a zero-width space count as non-printable, like str.isprintable
    assert_eq!(sanitize_upstream_error_body("a\u{a0}b\u{200b}c"), "a b c");
    assert_eq!(sanitize_upstream_error_body(""), "");
}

#[tokio::test]
async fn passthrough_log_upstream_failure() {
    assert_reported().await;
    assert_below_400().await;
    assert_broken_hook().await;
}

#[tokio::test]
async fn passthrough_log_upstream_failure_redacted() {
    assert_redacted().await;
    assert_redaction_settings();
}

#[tokio::test]
async fn passthrough_relay_failures() {
    assert_stream_error().await;
    assert_no_double_report().await;
    assert_broken_stream_hook().await;
}
