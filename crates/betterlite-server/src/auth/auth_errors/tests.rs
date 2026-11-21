#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_auth::auth_errors::handler::{AuthErrorSettings, LogSwitches};
use betterlite_auth::auth_errors::ports::{FailureHookCall, HookError};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tower::ServiceExt as _;
use tracing::subscriber::set_default;

use crate::routes;
use crate::routes::tests::state_for;
use crate::state::auth_errors::AuthFailureHook;

const ROUTE: &str = "/v1/files";
const MASTER_KEY: &str = "secret-master-key";
const DENIED_IP: &str = "203.0.113.9";

fn config(extra: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: {MASTER_KEY}\n  allowed_ips: [10.0.0.1]\n  \
         use_x_forwarded_for: true\n{extra}"
    ))
    .unwrap()
}

enum Reply {
    Replace,
    Raise,
}

struct Hook {
    reply: Reply,
    // route, metadata and the original failure the callback was handed
    seen: Mutex<Vec<(String, Option<Json>, String)>>,
}

impl Hook {
    fn replying(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl AuthFailureHook for Hook {
    async fn transform(&self, call: FailureHookCall<'_>) -> Result<Option<AuthFailure>, HookError> {
        self.seen.lock().unwrap().push((
            call.route.to_owned(),
            call.request_data.get("metadata").cloned(),
            call.original.to_string(),
        ));
        match self.reply {
            Reply::Replace => Ok(Some(AuthFailure::Http {
                detail: "rejected by a callback".to_owned(),
                param: None,
                status_code: 429,
                invalid_virtual_key: false,
            })),
            Reply::Raise => Err(HookError::Raised),
        }
    }
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

async fn send(hook: Option<&Arc<Hook>>, key: &str) -> (StatusCode, String, bool) {
    send_with(hook, key, "").await
}

async fn send_with(hook: Option<&Arc<Hook>>, key: &str, extra: &str) -> (StatusCode, String, bool) {
    let state = state_for(&config(extra));
    if let Some(installed) = hook
        && state.auth_errors.hook.set(Arc::clone(installed)).is_err()
    {
        panic!("a fresh slot takes the hook");
    }
    let reached = Arc::new(AtomicBool::new(false));
    let application = routes::router(
        state,
        Arc::new(AtomicBool::new(false)),
        Router::new().route(ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::clone(&reached)));
    let request = Request::builder()
        .method("POST")
        .uri(ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, "curl/8.5")
        .header("x-forwarded-for", DENIED_IP)
        .body(Body::from(r#"{"model":"gpt-4o"}"#))
        .unwrap();
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (status, text, reached.load(Ordering::Acquire))
}

#[test]
fn stage_is_registered() {
    let switches = "  allow_requests_on_db_unavailable: true\nlitellm_settings:\n  \
                    log_auth_failure_key_identity: true\n  log_client_error_tracebacks: true\n";
    let slot = &state_for(&config(switches)).auth_errors;
    let expected = AuthErrorSettings {
        log: LogSwitches {
            client_error_tracebacks: true,
            key_identity: true,
        },
        allow_requests_on_db_unavailable: true,
        use_x_forwarded_for: true,
    };
    assert_eq!(slot.settings, expected);
    assert!(slot.routes.is_some() && slot.redactor.is_some());
    assert!(slot.hook.get().is_none());
    let defaults = &state_for(&config("")).auth_errors.settings;
    assert_eq!(
        (defaults.log, defaults.allow_requests_on_db_unavailable),
        (LogSwitches::default(), false)
    );
}

#[tokio::test]
async fn failure_stops_downstream() {
    let hook = Hook::replying(Reply::Replace);
    let (status, text, reached) = send(Some(&hook), MASTER_KEY).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(!reached);
    let expected = r#"{"error":{"message":"rejected by a callback","type":"auth_error","param":"None","code":"429"}}"#;
    assert_eq!(text, expected);
    let seen = hook.seen.lock().unwrap();
    let Some((route, metadata, original)) = seen.first() else {
        panic!("the failure hook runs once");
    };
    assert_eq!((seen.len(), route.as_str()), (1, ROUTE));
    let client = BTreeMap::from([
        ("requester_ip_address", Json::Text(DENIED_IP.to_owned())),
        ("user_agent", Json::Text("curl/8.5".to_owned())),
    ]);
    let Some(Json::Object(stamped)) = metadata else {
        panic!("the hook sees the client context");
    };
    let ordered: BTreeMap<&str, Json> = stamped
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect();
    assert_eq!(ordered, client);
    assert!(original.contains("Access forbidden: IP address 203.0.113.9"));
}

#[tokio::test]
async fn outage_setting_never_lets_an_unmatched_key_through() {
    let hook = Hook::replying(Reply::Raise);
    let outage = "  allow_requests_on_db_unavailable: true\n";
    let (status, _, reached) = send_with(Some(&hook), "sk-wrong-key", outage).await;
    assert_eq!(
        (status, reached),
        (StatusCode::INTERNAL_SERVER_ERROR, false)
    );
    assert_eq!(hook.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn raising_hook_answers_500_without_the_key() {
    let hook = Hook::replying(Reply::Raise);
    let (status, text, reached) = send(Some(&hook), MASTER_KEY).await;
    assert_eq!(
        (status, text.as_str(), reached),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            false
        )
    );
}

#[tokio::test]
async fn without_a_hook_the_raised_failure_is_answered_and_logged() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let (status, text, reached) = send(None, MASTER_KEY).await;
    drop(guard);
    assert_eq!((status, reached), (StatusCode::FORBIDDEN, false));
    assert!(text.contains(r#""code":"403""#), "{text}");
    let output = read_to_string(log.path()).unwrap();
    let line = "ERROR LiteLLM Proxy: litellm.proxy.proxy_server.user_api_key_auth(): \
                Exception occured - Access forbidden: IP address 203.0.113.9";
    assert!(output.contains(line), "{output}");
    assert!(output.contains("requester_ip=\"203.0.113.9\""), "{output}");
    assert!(!output.contains(MASTER_KEY), "{output}");
}

#[rstest]
#[case::invalid_key("sk-wrong-key", Some("the API key is not valid"))]
#[case::missing_key("", None)]
#[tokio::test]
async fn invalid_key_reaches_the_hook_and_a_missing_key_does_not(
    #[case] key: &str,
    #[case] original: Option<&str>,
) {
    let hook = Hook::replying(Reply::Replace);
    let (_, _, reached) = send(Some(&hook), key).await;
    assert!(!reached);
    let seen: Vec<String> = hook
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|(_, _, failure)| failure.clone())
        .collect();
    assert_eq!(
        seen,
        original.map(str::to_owned).into_iter().collect::<Vec<_>>()
    );
}
