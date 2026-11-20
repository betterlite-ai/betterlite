#![cfg(test)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{RequestBodySafetyStageInput, run_stage};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const GRANT_SETTINGS: &str = "model_list:\n  - model_name: grant-model\n    litellm_params:\n      model: openai/gpt-4o\n      configurable_clientside_auth_params: [aws_role_name]\n";

fn config(extra: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&format!(
        "general_settings:\n  master_key: secret-master-key\n{extra}"
    ))
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::RequestBodySafety));

    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = RequestBodySafetyStageInput {
        context: master_context(&master_key),
        content_type: Some("application/json".to_owned()),
        body: Bytes::from_static(
            br#"{"model":"gpt-4o","metadata":{"langfuse_host":"https://evil.example"}}"#,
        ),
        slot: Some(state_for(&config("")).request_body_safety.clone()),
    };
    let input = StageInput::RequestBodySafety(Box::new(stage));
    let Err(StageFailure::RequestBodySafety(exception)) =
        dispatch(AuthBackend::RequestBodySafety, &input, &master_key)
    else {
        panic!("the production stage refuses a nested observability host");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
    assert!(
        exception.message.starts_with(
            "Authentication Error, Rejected Request: langfuse_host is not allowed in request body."
        ),
        "{}",
        exception.message
    );
    assert!(!exception.message.contains("evil.example"));

    // a non-text model answers the empty group name, so a deployment grant cannot admit the field
    let granted = RequestBodySafetyStageInput {
        context: master_context(&master_key),
        content_type: Some("application/json".to_owned()),
        body: Bytes::from_static(br#"{"model":["grant-model"],"aws_role_name":"admin"}"#),
        slot: Some(
            state_for(&config(GRANT_SETTINGS))
                .request_body_safety
                .clone(),
        ),
    };
    let granted_input = StageInput::RequestBodySafety(Box::new(granted));
    let Err(StageFailure::RequestBodySafety(non_text)) =
        dispatch(AuthBackend::RequestBodySafety, &granted_input, &master_key)
    else {
        panic!("a non-text model cannot use the deployment grant");
    };
    assert!(
        non_text
            .message
            .contains("aws_role_name is not allowed in request body"),
        "{}",
        non_text.message
    );

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::RequestBodySafety, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));

    // a body with nothing to scan still forwards the authenticated context
    let clean = RequestBodySafetyStageInput {
        context: master_context(&master_key),
        content_type: Some("application/json".to_owned()),
        body: Bytes::from_static(b"not json"),
        slot: Some(state_for(&config("")).request_body_safety.clone()),
    };
    let Ok(forwarded) = run_stage(&clean) else {
        panic!("an unscannable body passes");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    // JSON under a binary content type still parses, like _parse_binary_body
    let binary = RequestBodySafetyStageInput {
        context: master_context(&master_key),
        content_type: Some("application/x-protobuf".to_owned()),
        body: Bytes::from_static(br#"{"api_base":"https://evil.example"}"#),
        slot: Some(state_for(&config("")).request_body_safety.clone()),
    };
    let binary_input = StageInput::RequestBodySafety(Box::new(binary));
    let Err(StageFailure::RequestBodySafety(binary_error)) =
        dispatch(AuthBackend::RequestBodySafety, &binary_input, &master_key)
    else {
        panic!("a JSON protobuf body is still scanned");
    };
    assert!(
        binary_error
            .message
            .contains("api_base is not allowed in request body"),
        "{}",
        binary_error.message
    );
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>, body: Bytes) -> String {
    reached.store(true, Ordering::Release);
    String::from_utf8(body.to_vec()).unwrap_or_default()
}

fn application(config: &Config, reached: &Arc<AtomicBool>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(Arc::clone(reached)))
}

fn request(content_type: &'static str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("the request builds")
}

async fn run_request_with(
    config: &Config,
    content_type: &'static str,
    body: &'static str,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(config, &reached)
        .oneshot(request(content_type, body))
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(bytes.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

// A body whose stream fails on the first frame, so the middleware's body read cannot finish.
struct BrokenBody;

impl http_body::Body for BrokenBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        Poll::Ready(Some(Err(std::io::Error::other("the body stream fails"))))
    }
}

#[tokio::test]
async fn unreadable_bodies_are_buffered_after_authentication() {
    // a broken body under a rejected key never reaches the body read, so the master key stage answers first
    let rejected = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer wrong-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::new(BrokenBody))
        .expect("the request builds");
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&config(""), &reached)
        .oneshot(rejected)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!reached.load(Ordering::Acquire));

    // under the master key the failed read becomes the buffering 400 and the handler never runs
    let broken = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::new(BrokenBody))
        .expect("the request builds");
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(&config(""), &reached)
        .oneshot(broken)
        .await
        .expect("the request reaches the router");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    assert!(text.contains("Failed to buffer the request body"), "{text}");
    assert!(!reached.load(Ordering::Acquire));
}

#[tokio::test]
async fn failure_stops_downstream() {
    // the banned field is refused before provider dispatch, with the public error and no echoed value
    let (status, body, reached) = run_request_with(
        &config(""),
        "application/json",
        r#"{"model":"gpt-4o","api_base":"https://evil.example"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(
        body.contains("api_base is not allowed in request body"),
        "{body}"
    );
    assert!(!body.contains("evil.example"), "{body}");
    assert!(!reached);

    // the proxy-wide credential opt-in still cannot retarget a URL-valued field
    let (opted_status, opted_body, opted_reached) = run_request_with(
        &config("  allow_client_side_credentials: true\n"),
        "application/json",
        r#"{"model":"gpt-4o","api_base":"https://evil.example"}"#,
    )
    .await;
    assert_eq!(opted_status, StatusCode::UNAUTHORIZED);
    assert!(!opted_body.contains("evil.example"), "{opted_body}");
    assert!(!opted_reached);

    // the deployment grant admits only its own field, and no other deployment's model inherits it
    let (granted_status, granted_body, granted_reached) = run_request_with(
        &config(GRANT_SETTINGS),
        "application/json",
        r#"{"model":"grant-model","aws_role_name":"admin"}"#,
    )
    .await;
    assert_eq!(granted_status, StatusCode::OK);
    assert!(granted_reached);
    assert_eq!(
        granted_body,
        r#"{"model":"grant-model","aws_role_name":"admin"}"#
    );
    let (other_status, _other_body, other_reached) = run_request_with(
        &config(GRANT_SETTINGS),
        "application/json",
        r#"{"model":"other-model","aws_role_name":"admin"}"#,
    )
    .await;
    assert_eq!(other_status, StatusCode::UNAUTHORIZED);
    assert!(!other_reached);
    let (paired_status, _paired_body, paired_reached) = run_request_with(
        &config(GRANT_SETTINGS),
        "application/json",
        r#"{"model":"grant-model","aws_role_name":"admin","api_base":"https://evil.example"}"#,
    )
    .await;
    assert_eq!(paired_status, StatusCode::UNAUTHORIZED);
    assert!(!paired_reached);

    // a clean request reaches the handler with its body restored byte for byte
    let clean = r#"{"model":"gpt-4o","messages":[]}"#;
    let (clean_status, clean_body, clean_reached) =
        run_request_with(&config(""), "application/json", clean).await;
    assert_eq!(clean_status, StatusCode::OK);
    assert!(clean_reached);
    assert_eq!(clean_body, clean);
}

#[tokio::test]
async fn form_fields_are_scanned() {
    // dict(request.form()): a urlencoded field reaches the same banned check as a JSON key, bracket form included
    let (status, body, reached) = run_request_with(
        &config(""),
        "application/x-www-form-urlencoded",
        "model=gpt-4o&metadata%5Blangfuse_host%5D=https%3A%2F%2Fevil.example",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("langfuse_host is not allowed in request body"),
        "{body}"
    );
    assert!(!body.contains("evil.example"), "{body}");
    assert!(!reached);

    // an upload part under a bracket-form metadata key is scanned by its top-level name
    let bracket = "--safety-boundary\r\nContent-Disposition: form-data; name=\"metadata[langfuse_host]\"; filename=\"host.txt\"\r\nContent-Type: text/plain\r\n\r\nhttps://evil.example\r\n--safety-boundary--\r\n";
    let (bracket_status, bracket_body, bracket_reached) = run_request_with(
        &config(""),
        "multipart/form-data; boundary=safety-boundary",
        bracket,
    )
    .await;
    assert_eq!(bracket_status, StatusCode::UNAUTHORIZED);
    assert!(
        bracket_body.contains("langfuse_host is not allowed in request body"),
        "{bracket_body}"
    );
    assert!(!bracket_reached);

    // an upload part keeps its key, so a multipart api_base is refused on presence alone
    let multipart = "--safety-boundary\r\nContent-Disposition: form-data; name=\"api_base\"; filename=\"base.txt\"\r\nContent-Type: text/plain\r\n\r\nhttps://evil.example\r\n--safety-boundary--\r\n";
    let (upload_status, upload_body, upload_reached) = run_request_with(
        &config(""),
        "multipart/form-data; boundary=safety-boundary",
        multipart,
    )
    .await;
    assert_eq!(upload_status, StatusCode::UNAUTHORIZED);
    assert!(
        upload_body.contains("api_base is not allowed in request body"),
        "{upload_body}"
    );
    assert!(!upload_body.contains("evil.example"), "{upload_body}");
    assert!(!upload_reached);

    // a form post with no banned field forwards with its body intact
    let clean = "model=gpt-4o&messages=%5B%5D";
    let (clean_status, clean_body, clean_reached) =
        run_request_with(&config(""), "application/x-www-form-urlencoded", clean).await;
    assert_eq!(clean_status, StatusCode::OK);
    assert!(clean_reached);
    assert_eq!(clean_body, clean);

    // a form body the decoder cannot read is not scanned here, so the route's own decoder reports it
    let (broken_status, _broken_body, broken_reached) = run_request_with(
        &config(""),
        "multipart/form-data; boundary=missing",
        "not a multipart body",
    )
    .await;
    assert_eq!(broken_status, StatusCode::OK);
    assert!(broken_reached);

    // a form metadata value that is not JSON is left to the route too, where LiteLLM's pre-read raises
    let (metadata_status, _metadata_body, metadata_reached) = run_request_with(
        &config(""),
        "application/x-www-form-urlencoded",
        "model=gpt-4o&metadata=not-json",
    )
    .await;
    assert_eq!(metadata_status, StatusCode::OK);
    assert!(metadata_reached);
}
