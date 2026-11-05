#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{IntoFuture as _, pending};
use std::time::Duration;

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse as _, Response};
use rstest::rstest;
use serde_json::value::RawValue;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};

use super::support::{FakeResolver, ResolverFailure, manager, resolver};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{ActionRequest, AwsOptionalParams, AwsRequestError};
use crate::secret_contracts::options::SecretTimeout;

struct Captured {
    head: Parts,
    body: String,
}

#[derive(Clone)]
struct Answer {
    status: StatusCode,
    body: Vec<u8>,
    hang: bool,
    captured: mpsc::Sender<Captured>,
}

async fn answer(State(answer): State<Answer>, request: Request) -> Response {
    let (head, body) = request.into_parts();
    let bytes = to_bytes(body, 0x0001_0000).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    answer
        .captured
        .send(Captured { head, body: text })
        .await
        .unwrap();
    if answer.hang {
        pending::<()>().await;
    }
    (answer.status, answer.body).into_response()
}

async fn with_server<Call, Make>(
    status: u16,
    body: Vec<u8>,
    hang: bool,
    call: Make,
) -> (Call::Output, Vec<Captured>)
where
    Make: FnOnce(String) -> Call,
    Call: Future,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/bedrock-runtime", listener.local_addr().unwrap());
    let (captured, mut received) = mpsc::channel(16);
    let (stop, stopped) = oneshot::channel();
    let router = Router::new().fallback(answer).with_state(Answer {
        status: StatusCode::from_u16(status).unwrap(),
        body,
        hang,
        captured,
    });
    let serving = axum::serve(listener, router)
        .with_graceful_shutdown(async { stopped.await.unwrap() })
        .into_future();
    let caller = async {
        let output = call(endpoint).await;
        stop.send(()).unwrap();
        output
    };
    let (output, shutdown) = tokio::join!(caller, serving);
    shutdown.unwrap();
    let mut requests = Vec::new();
    while let Ok(request) = received.try_recv() {
        requests.push(request);
    }
    (output, requests)
}

fn client_for(endpoint: String) -> AwsSecretsManager<FakeResolver> {
    manager(
        FakeResolver {
            endpoint: Some(endpoint),
            ..resolver("us-east-1")
        },
        BTreeMap::new(),
    )
}

fn get_request(data: Option<&BTreeMap<String, Box<RawValue>>>) -> ActionRequest<'_> {
    ActionRequest {
        action: "GetSecretValue",
        secret_name: "my/secret",
        request_data: data,
    }
}

#[tokio::test]
async fn posts_the_signed_request_and_returns_the_json_object() {
    let (response, requests) = with_server(
        200,
        br#"{"Name":"my/secret","SecretString":"hunter2"}"#.to_vec(),
        false,
        |endpoint| async move {
            client_for(endpoint)
                .async_post_action(&get_request(None), None, None)
                .await
                .unwrap()
        },
    )
    .await;
    assert_eq!(response.fields["SecretString"].get(), r#""hunter2""#);
    assert_eq!(response.fields["Name"].get(), r#""my/secret""#);
    let [request] = requests.as_slice() else {
        panic!("expected exactly one request, got {}", requests.len());
    };
    assert_eq!(request.head.method, "POST");
    assert_eq!(request.head.uri.path(), "/secretsmanager");
    assert_eq!(request.body, r#"{"SecretId":"my/secret"}"#);
    let headers = &request.head.headers;
    assert_eq!(headers["content-type"], "application/x-amz-json-1.1");
    assert_eq!(headers["x-amz-target"], "secretsmanager.GetSecretValue");
    assert_eq!(headers["x-amz-security-token"], "session-token");
    assert!(
        headers["authorization"]
            .to_str()
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
    );
    assert!(headers.contains_key("x-amz-date"));
}

#[tokio::test]
async fn posts_request_data_as_the_body() {
    let mut data = BTreeMap::new();
    data.insert(
        "SecretId".to_owned(),
        RawValue::from_string(r#""my/secret""#.to_owned()).unwrap(),
    );
    data.insert(
        "VersionStage".to_owned(),
        RawValue::from_string(r#""AWSPREVIOUS""#.to_owned()).unwrap(),
    );
    let (_response, requests) = with_server(200, b"{}".to_vec(), false, |endpoint| async move {
        client_for(endpoint)
            .async_post_action(&get_request(Some(&data)), None, None)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(
        requests[0].body,
        r#"{"SecretId":"my/secret","VersionStage":"AWSPREVIOUS"}"#
    );
}

#[tokio::test]
async fn optional_params_reach_the_credential_resolver() {
    let mut params = AwsOptionalParams {
        aws_profile_name: Some("caller-profile".to_owned()),
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_bedrock_runtime_endpoint: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_region_name: None,
        aws_external_id: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
    };
    let (seen, _requests) = with_server(200, b"{}".to_vec(), false, |endpoint| async move {
        let client = client_for(endpoint);
        client
            .async_post_action(&get_request(None), Some(&mut params), None)
            .await
            .unwrap();
        client.dependencies.resolver.seen.lock().unwrap().clone()
    })
    .await;
    assert_eq!(
        seen[0].0.get("profile").map(String::as_str),
        Some("caller-profile")
    );
}

#[rstest]
#[case::bad_request(
    400,
    r#"{"__type":"ResourceNotFoundException","message":"can't find it"}"#
)]
#[case::server_error(500, "internal failure")]
#[case::redirect_status(302, "moved")]
#[tokio::test]
async fn an_error_status_becomes_a_message_with_the_response_text(
    #[case] status: u16,
    #[case] text: &str,
) {
    let (error, _requests) = with_server(
        status,
        text.as_bytes().to_vec(),
        false,
        |endpoint| async move {
            client_for(endpoint)
                .async_post_action(&get_request(None), None, None)
                .await
                .unwrap_err()
        },
    )
    .await;
    assert!(matches!(error, AwsRequestError::Http { .. }));
    assert_eq!(error.to_string(), format!("HTTP error occurred: {text}"));
}

#[rstest]
#[case::total(SecretTimeout::Total(Duration::from_millis(100)))]
#[case::every_phase(SecretTimeout::Phases {
    connect: Some(Duration::from_millis(25)),
    read: Some(Duration::from_millis(25)),
    write: Some(Duration::from_millis(25)),
    pool: Some(Duration::from_millis(25)),
})]
#[tokio::test]
async fn a_slow_server_becomes_a_timeout_message(#[case] timeout: SecretTimeout) {
    let (error, _requests) = with_server(200, b"{}".to_vec(), true, |endpoint| async move {
        client_for(endpoint)
            .async_post_action(&get_request(None), None, Some(timeout))
            .await
            .unwrap_err()
    })
    .await;
    assert!(matches!(error, AwsRequestError::Timeout));
    assert_eq!(error.to_string(), "Timeout error occurred");
}

#[rstest]
#[case::not_json(b"plain text".to_vec())]
#[case::not_an_object(b"[1,2]".to_vec())]
#[case::empty(Vec::new())]
#[tokio::test]
async fn a_success_body_that_is_not_a_json_object_is_a_response_error(#[case] body: Vec<u8>) {
    let (error, _requests) = with_server(200, body, false, |endpoint| async move {
        client_for(endpoint)
            .async_post_action(&get_request(None), None, None)
            .await
            .unwrap_err()
    })
    .await;
    assert!(matches!(error, AwsRequestError::Response(_)));
}

#[tokio::test]
async fn a_response_past_the_size_limit_is_refused() {
    let mut oversized = br#"{"SecretString":""#.to_vec();
    oversized.resize(0x0010_0001, b'a');
    oversized.extend_from_slice(br#""}"#);
    let (error, _requests) = with_server(200, oversized, false, |endpoint| async move {
        client_for(endpoint)
            .async_post_action(&get_request(None), None, None)
            .await
            .unwrap_err()
    })
    .await;
    assert!(matches!(
        error,
        AwsRequestError::ResponseTooLarge { limit: 0x0010_0000 }
    ));
}

#[tokio::test]
async fn an_unreachable_server_is_a_transport_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/bedrock-runtime", listener.local_addr().unwrap());
    drop(listener);
    let error = client_for(endpoint)
        .async_post_action(&get_request(None), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AwsRequestError::Transport(_)));
}

#[tokio::test]
async fn a_credential_failure_stops_before_any_request_is_sent() {
    let (error, requests) = with_server(200, b"{}".to_vec(), false, |endpoint| async move {
        let failing = FakeResolver {
            endpoint: Some(endpoint),
            fail: true,
            ..resolver("us-east-1")
        };
        manager(failing, BTreeMap::new())
            .async_post_action(&get_request(None), None, None)
            .await
            .unwrap_err()
    })
    .await;
    assert!(matches!(
        error,
        AwsRequestError::Credentials(ResolverFailure::Unavailable)
    ));
    assert!(requests.is_empty());
}
