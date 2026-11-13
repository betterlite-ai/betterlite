#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::mem::take;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use serde::{Deserialize, Serialize};

use super::support::{
    FakeResolver, FakeTransport, ResolverFailure, answer, get_secret_request, manager, resolver,
    transport,
};
use crate::aws::AwsSecretsManager;
use crate::aws::request_signing::{
    ActionRequest, AwsOptionalParams, AwsRequestError, TransportError,
};
use crate::secret_contracts::options::SecretTimeout;

const ENDPOINT: &str = "http://127.0.0.1:9/bedrock-runtime";

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Reply {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "SecretString")]
    secret_string: Option<String>,
}

#[derive(Serialize)]
struct VersionBody {
    #[serde(rename = "SecretId")]
    secret_id: &'static str,
    #[serde(rename = "VersionStage")]
    version_stage: &'static str,
    #[serde(rename = "SecretString")]
    secret_string: &'static str,
}

fn client_for(transport: &Arc<FakeTransport>) -> AwsSecretsManager<FakeResolver, FakeTransport> {
    let mut client = manager(
        FakeResolver {
            endpoint: Some(ENDPOINT.to_owned()),
            ..resolver("us-east-1")
        },
        BTreeMap::new(),
    );
    client.dependencies.transport = Arc::<FakeTransport>::clone(transport);
    client
}

#[tokio::test]
async fn posts_the_signed_request_and_returns_the_json_object() {
    let sending = answer(200, br#"{"Name":"my/secret","SecretString":"hunter2"}"#);
    let response: Reply = client_for(&sending)
        .async_post_action(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap();
    assert_eq!(response.secret_string.as_deref(), Some("hunter2"));
    assert_eq!(response.name.as_deref(), Some("my/secret"));
    let sent = take(&mut *sending.sent.lock().unwrap());
    let [request] = sent.as_slice() else {
        panic!("expected exactly one request, got {}", sent.len());
    };
    assert_eq!(request.url, "http://127.0.0.1:9/secretsmanager");
    assert_eq!(request.body, br#"{"SecretId":"my/secret"}"#);
    let headers = &request.headers;
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
    let data = VersionBody {
        secret_id: "my/secret",
        version_stage: "AWSPREVIOUS",
        secret_string: "hunter2",
    };
    let request = ActionRequest {
        action: "GetSecretValue",
        secret_name: "my/secret",
        request_data: Some(&data),
    };
    let sending = answer(200, b"{}");
    let _reply: Reply = client_for(&sending)
        .async_post_action(&request, None, None)
        .await
        .unwrap();
    assert_eq!(
        sending.sent.lock().unwrap()[0].body,
        br#"{"SecretId":"my/secret","VersionStage":"AWSPREVIOUS","SecretString":"hunter2"}"#
    );
}

#[tokio::test]
async fn the_debug_text_of_a_sent_request_hides_the_body() {
    let data = VersionBody {
        secret_id: "my/secret",
        version_stage: "AWSPREVIOUS",
        secret_string: "hunter2",
    };
    let request = ActionRequest {
        action: "GetSecretValue",
        secret_name: "my/secret",
        request_data: Some(&data),
    };
    let sending = answer(200, b"{}");
    let _reply: Reply = client_for(&sending)
        .async_post_action(&request, None, None)
        .await
        .unwrap();
    let transport_text = take(&mut sending.sent.lock().unwrap()[0].debug_text);
    let request_text = format!("{request:?}");
    assert!(!transport_text.contains("hunter2"));
    assert!(transport_text.contains("body_bytes: 78"));
    assert_eq!(
        request_text,
        r#"ActionRequest { action: "GetSecretValue", secret_name: "my/secret", has_request_data: true }"#
    );
}

#[tokio::test]
async fn the_signature_is_dated_by_the_injected_clock() {
    let sending = answer(200, b"{}");
    let _reply: Reply = client_for(&sending)
        .async_post_action(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap();
    assert_eq!(
        sending.sent.lock().unwrap()[0].headers["x-amz-date"],
        "20240305T123045Z"
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
    let client = client_for(&answer(200, b"{}"));
    client
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), Some(&mut params), None)
        .await
        .unwrap();
    let seen = client.dependencies.resolver.seen.lock().unwrap().clone();
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
    let error = client_for(&answer(status, text.as_bytes()))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AwsRequestError::Http { .. }));
    assert_eq!(error.to_string(), format!("HTTP error occurred: {text}"));
}

#[rstest]
#[case::total(
    Some(SecretTimeout::Total(Duration::from_millis(100))),
    Duration::from_millis(100)
)]
#[case::every_phase(
    Some(SecretTimeout::Phases {
        connect: Some(Duration::from_millis(25)),
        read: Some(Duration::from_millis(25)),
        write: Some(Duration::from_millis(25)),
        pool: Some(Duration::from_millis(25)),
    }),
    Duration::from_millis(100)
)]
#[case::an_unset_phase(
    Some(SecretTimeout::Phases {
        connect: Some(Duration::from_millis(25)),
        read: None,
        write: Some(Duration::from_millis(25)),
        pool: Some(Duration::from_millis(25)),
    }),
    Duration::from_secs(600)
)]
#[case::none(None, Duration::from_secs(600))]
#[tokio::test]
async fn the_timeout_option_reaches_the_transport(
    #[case] timeout: Option<SecretTimeout>,
    #[case] expected: Duration,
) {
    let sending = answer(200, b"{}");
    client_for(&sending)
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, timeout)
        .await
        .unwrap();
    assert_eq!(sending.sent.lock().unwrap()[0].timeout, expected);
}

#[tokio::test]
async fn a_slow_server_becomes_a_timeout_message() {
    let error = client_for(&transport(Err(TransportError::Timeout)))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AwsRequestError::Timeout));
    assert_eq!(error.to_string(), "Timeout error occurred");
}

#[rstest]
#[case::not_json(b"plain text".to_vec())]
#[case::not_an_object(b"[1,2]".to_vec())]
#[case::empty(Vec::new())]
#[tokio::test]
async fn a_success_body_that_is_not_a_json_object_is_a_response_error(#[case] body: Vec<u8>) {
    let error = client_for(&answer(200, &body))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AwsRequestError::Response(_)));
}

#[tokio::test]
async fn a_response_past_the_size_limit_is_refused() {
    let mut oversized = br#"{"SecretString":""#.to_vec();
    oversized.resize(0x0010_0001, b'a');
    oversized.extend_from_slice(br#""}"#);
    let sending = answer(200, &oversized);
    let error = client_for(&sending)
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AwsRequestError::ResponseTooLarge { limit: 0x0010_0000 }
    ));
    assert_eq!(
        sending.sent.lock().unwrap()[0].max_response_bytes,
        0x0010_0000
    );
}

#[tokio::test]
async fn a_transport_that_stops_reading_at_the_limit_is_a_size_error() {
    let error = client_for(&transport(Err(TransportError::ResponseTooLarge)))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AwsRequestError::ResponseTooLarge { limit: 0x0010_0000 }
    ));
}

#[tokio::test]
async fn an_unreachable_server_is_a_transport_error() {
    let refused = TransportError::Failed(io::Error::from(io::ErrorKind::ConnectionRefused));
    let error = client_for(&transport(Err(refused)))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    let AwsRequestError::Transport(source) = error else {
        panic!("expected a transport error, got {error}");
    };
    assert_eq!(source.kind(), io::ErrorKind::ConnectionRefused);
}

#[tokio::test]
async fn a_credential_failure_stops_before_any_request_is_sent() {
    let sending = answer(200, b"{}");
    let failing = FakeResolver {
        endpoint: Some(ENDPOINT.to_owned()),
        fail: true,
        ..resolver("us-east-1")
    };
    let mut client = manager(failing, BTreeMap::new());
    client.dependencies.transport = Arc::<FakeTransport>::clone(&sending);
    let error = client
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AwsRequestError::Credentials(ResolverFailure::Unavailable)
    ));
    assert!(sending.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_response_of_exactly_the_size_limit_is_accepted() {
    let mut body = br#"{"SecretString":""#.to_vec();
    body.resize(0x0010_0000 - 2, b'a');
    body.extend_from_slice(br#""}"#);
    assert_eq!(body.len(), 0x0010_0000);
    let response = client_for(&answer(200, &body))
        .async_post_action::<_, Reply>(&get_secret_request("my/secret"), None, None)
        .await
        .unwrap();
    assert_eq!(
        response.secret_string.map(|value| value.len()),
        Some(0x0010_0000 - 19)
    );
}
