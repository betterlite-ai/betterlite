#![cfg(test)]

use std::future::{IntoFuture as _, pending};
use std::io;
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::post;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::{JevAnswer, JevSystemOneResponse, MAX_RESPONSE_BYTES, call_systemone};
use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::fixtures::{guardrail, json, settings};
use crate::typesafe::typesafe_failure_handling::TypeSafeError;
use crate::typesafe::typesafe_instructions::question_instructions;
use crate::typesafe::typesafe_state_building::{TypeSafeExchange, TypeSafeState};
use crate::typesafe::typesafe_tool_extraction::ToolCallEntry;
use crate::typesafe::{DEFAULT_JEV_TIMEOUT, TypeSafeGuardrail};

fn state() -> TypeSafeState {
    let mut tool_exchanges = IndexMap::new();
    tool_exchanges.insert(
        "e0".to_owned(),
        TypeSafeExchange {
            tool_calls: vec![ToolCallEntry {
                name: Some(json(r#""lookup""#)),
                arguments: None,
            }],
            result: "result".to_owned(),
        },
    );
    TypeSafeState {
        task: "task".to_owned(),
        system: "system".to_owned(),
        tool_exchanges,
    }
}

fn guardrail_at(base: &str, fallback: UnreachableFallback) -> TypeSafeGuardrail {
    let mut settings = settings();
    settings.api_base = base.to_owned();
    settings.unreachable_fallback = fallback;
    guardrail(settings)
}

async fn serve_router(router: Router) -> (String, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, server)
}

async fn serve(status: StatusCode, body: String) -> (String, JoinSet<io::Result<()>>) {
    let router = Router::new().route(
        "/v1/systemone",
        post(move || {
            let reply = body.clone();
            async move { (status, reply) }
        }),
    );
    serve_router(router).await
}

async fn serve_never_answering() -> (String, JoinSet<io::Result<()>>) {
    let router = Router::new().route("/v1/systemone", post(|| async { pending::<()>().await }));
    serve_router(router).await
}

struct CapturedRequest {
    path: String,
    headers: HeaderMap,
    body: String,
}

async fn serve_capturing(
    status: StatusCode,
    body: String,
) -> (
    String,
    mpsc::Receiver<CapturedRequest>,
    JoinSet<io::Result<()>>,
) {
    let (sender, receiver) = mpsc::channel(1);
    let router = Router::new().route(
        "/v1/systemone",
        post(move |uri: Uri, headers: HeaderMap, request_body: String| {
            let request_sender = sender.clone();
            let reply = body.clone();
            async move {
                drop(
                    request_sender
                        .send(CapturedRequest {
                            path: uri.path().to_owned(),
                            headers,
                            body: request_body,
                        })
                        .await,
                );
                (status, reply)
            }
        }),
    );
    let (base, server) = serve_router(router).await;
    (base, receiver, server)
}

async fn call_against(
    status: StatusCode,
    body: String,
    fallback: UnreachableFallback,
) -> Result<Option<JevSystemOneResponse>, TypeSafeError> {
    let (base, mut server) = serve(status, body).await;
    let guardrail = guardrail_at(&base, fallback);
    let result = call_systemone(&guardrail, &state(), &["e0".to_owned()]).await;
    server.shutdown().await;
    result
}

fn assert_request(captured: &CapturedRequest) {
    assert_eq!(captured.path, "/v1/systemone");
    assert_eq!(
        captured
            .headers
            .get("authorization")
            .expect("authorization")
            .to_str()
            .expect("text"),
        "Bearer secret-key"
    );
    assert_eq!(
        captured
            .headers
            .get("content-type")
            .expect("content type")
            .to_str()
            .expect("text"),
        "application/json"
    );
    assert_body(&captured.body);
}

fn assert_body(request_body: &str) {
    let body = json(request_body);
    let fields = body.as_object().expect("request object");
    assert_eq!(
        fields.get("model").and_then(Json::as_str),
        Some("jev-latest")
    );
    assert_eq!(
        fields.get("state"),
        Some(&json(
            r#"{"task":"task","system":"system","tool_exchanges":{"e0":{"tool_calls":[{"name":"lookup","arguments":null}],"result":"result"}}}"#
        ))
    );
    let question = fields
        .get("questions")
        .and_then(Json::as_object)
        .and_then(|questions| questions.get("e0"))
        .and_then(Json::as_object)
        .expect("the e0 question");
    assert_eq!(question.get("type").and_then(Json::as_str), Some("noul"));
    assert_eq!(
        question.get("instructions").and_then(Json::as_str),
        Some(question_instructions("e0").as_str())
    );
}

#[tokio::test]
async fn call_systemone_posts_state_and_questions_to_the_systemone_path() {
    let (base, mut receiver, mut server) = serve_capturing(
        StatusCode::OK,
        r#"{"answers":{"e0":{"type":"noul","noul":0.1}}}"#.to_owned(),
    )
    .await;
    let guardrail = guardrail_at(&base, UnreachableFallback::FailOpen);
    let response = call_systemone(&guardrail, &state(), &["e0".to_owned()])
        .await
        .expect("fail open")
        .expect("a parsed response");
    assert_eq!(
        response.answers.get("e0"),
        Some(&JevAnswer::Noul { noul: 0.1_f64 })
    );
    let captured = receiver.recv().await.expect("the Jev API was called");
    assert_request(&captured);
    server.shutdown().await;
}

#[rstest]
#[case::plain(r#"{"answers":{"e0":{"type":"noul","noul":0.1}}}"#)]
#[case::extra_fields(
    r#"{"model":"jev","answers":{"e0":{"type":"noul","noul":0.9,"extra":1}},"usage":{}}"#
)]
#[tokio::test]
async fn call_systemone_parses_the_jev_response_shape(#[case] body: &str) {
    let result = call_against(
        StatusCode::OK,
        body.to_owned(),
        UnreachableFallback::FailOpen,
    )
    .await
    .expect("fail open")
    .expect("a parsed response");
    let JevAnswer::Noul { noul } = result.answers.get("e0").expect("e0 answer");
    assert!((0.0_f64..=1.0_f64).contains(noul));
}

#[tokio::test]
async fn call_systemone_fails_open_on_a_status_error() {
    let result = call_against(
        StatusCode::INTERNAL_SERVER_ERROR,
        "oops".to_owned(),
        UnreachableFallback::FailOpen,
    )
    .await;
    assert_eq!(result, Ok(None));
}

#[tokio::test]
async fn call_systemone_fails_closed_on_a_status_error() {
    let result = call_against(
        StatusCode::INTERNAL_SERVER_ERROR,
        "oops".to_owned(),
        UnreachableFallback::FailClosed,
    )
    .await;
    assert_eq!(
        result,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service returned an error".to_owned()
        })
    );
}

#[tokio::test]
async fn call_systemone_fails_open_on_an_unreadable_body() {
    let result = call_against(
        StatusCode::OK,
        "not json".to_owned(),
        UnreachableFallback::FailOpen,
    )
    .await;
    assert_eq!(result, Ok(None));
}

#[rstest]
#[case::unknown_answer_type(r#"{"answers":{"e0":{"type":"choice"}}}"#)]
#[case::missing_answers("{}")]
#[case::missing_noul(r#"{"answers":{"e0":{"type":"noul"}}}"#)]
#[case::noul_out_of_range(r#"{"answers":{"e0":{"type":"noul","noul":1.5}}}"#)]
#[tokio::test]
async fn call_systemone_fails_closed_on_an_unexpected_response_shape(#[case] body: &str) {
    let result = call_against(
        StatusCode::OK,
        body.to_owned(),
        UnreachableFallback::FailClosed,
    )
    .await;
    assert_eq!(
        result,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service returned unexpected response shape".to_owned()
        })
    );
}

#[tokio::test]
async fn call_systemone_fails_open_when_the_service_is_unreachable() {
    let guardrail = guardrail_at("http://127.0.0.1:1", UnreachableFallback::FailOpen);
    let result = call_systemone(&guardrail, &state(), &["e0".to_owned()]).await;
    assert_eq!(result, Ok(None));
}

#[tokio::test]
async fn call_systemone_fails_closed_when_the_service_is_unreachable() {
    let guardrail = guardrail_at("http://127.0.0.1:1", UnreachableFallback::FailClosed);
    let result = call_systemone(&guardrail, &state(), &["e0".to_owned()]).await;
    assert_eq!(
        result,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service request failed".to_owned()
        })
    );
}

// The handler never answers, so only the guardrail's own timeout can end either call.
#[tokio::test]
async fn call_systemone_applies_the_guardrail_timeout() {
    let (base, mut server) = serve_never_answering().await;
    let mut guardrail = guardrail_at(&base, UnreachableFallback::FailClosed);
    guardrail.timeout = Duration::from_millis(100);
    let closed = call_systemone(&guardrail, &state(), &["e0".to_owned()]).await;
    assert_eq!(
        closed,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service request failed".to_owned()
        })
    );
    guardrail.settings.unreachable_fallback = UnreachableFallback::FailOpen;
    let open = call_systemone(&guardrail, &state(), &["e0".to_owned()]).await;
    assert_eq!(open, Ok(None));
    server.shutdown().await;
}

#[tokio::test]
async fn call_systemone_rejects_an_oversized_response() {
    let body = " ".repeat(MAX_RESPONSE_BYTES + 1);
    assert_eq!(
        call_against(StatusCode::OK, body.clone(), UnreachableFallback::FailOpen).await,
        Ok(None)
    );
    assert_eq!(
        call_against(StatusCode::OK, body, UnreachableFallback::FailClosed).await,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service returned an oversized response".to_owned()
        })
    );
}

#[tokio::test]
async fn call_systemone_reads_a_body_at_the_size_limit() {
    let body = " ".repeat(MAX_RESPONSE_BYTES);
    assert_eq!(
        call_against(StatusCode::OK, body, UnreachableFallback::FailClosed).await,
        Err(TypeSafeError::Unavailable {
            message: "TypeSafe evaluation service returned an unreadable response".to_owned()
        })
    );
}

#[test]
fn the_jev_timeout_default_matches_litellm() {
    assert_eq!(DEFAULT_JEV_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn call_systemone_trims_a_trailing_slash_from_api_base() {
    let (base, mut receiver, mut server) =
        serve_capturing(StatusCode::OK, r#"{"answers":{}}"#.to_owned()).await;
    let trimmed = format!("{base}/");
    let guardrail = guardrail_at(&trimmed, UnreachableFallback::FailOpen);
    call_systemone(&guardrail, &state(), &["e0".to_owned()])
        .await
        .expect("fail open")
        .expect("an empty answers map");
    let captured = receiver.recv().await.expect("the Jev API was called");
    assert_eq!(captured.path, "/v1/systemone");
    server.shutdown().await;
}
