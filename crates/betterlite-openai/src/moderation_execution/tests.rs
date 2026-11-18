#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::chat::MockSettings;
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults};
use crate::chat::execution::ChatExecutor;
use crate::chat::response::ChatResponse;
use crate::moderation_request::ModerationRequest;
use crate::openai_base_config::CatalogCapabilities;

use super::{ModerationError, send};

const RESPONSE: &str = include_str!("../../fuzz/corpus/moderation_response/complete.json");

#[rstest]
#[case::versioned("/v1", "/v1/moderations")]
#[case::unversioned("", "/moderations")]
#[tokio::test]
async fn moderations_moderation_tests_behavior(#[case] base_path: &str, #[case] expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (executor, deployment) =
        moderation_executor(&format!("http://{address}{base_path}"), ", api_key: key");
    let request = ModerationRequest::parse(br#"{"input":"hello"}"#).expect("request");
    let (captured, result) = tokio::join!(
        answer(listener, "200 OK", RESPONSE),
        send(&executor, &deployment, &request)
    );
    let outcome = result.expect("moderation answer");
    assert_eq!(outcome.response.id, "modr-1");
    assert_eq!(outcome.upstream.status, 200);
    let (head, body) = captured;
    assert!(
        head.starts_with(&format!("POST {expected} HTTP/1.1")),
        "{head}"
    );
    assert!(head.contains("authorization: Bearer key"), "{head}");
    assert!(head.contains("content-type: application/json"), "{head}");
    assert_eq!(
        body,
        br#"{"input":"hello","model":"omni-moderation-latest"}"#
    );
}

#[tokio::test]
async fn a_provider_error_keeps_the_status_and_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (executor, deployment) =
        moderation_executor(&format!("http://{address}/v1"), ", api_key: key");
    let request = ModerationRequest::parse(br#"{"input":"hello"}"#).expect("request");
    let (_, result) = tokio::join!(
        answer(
            listener,
            "400 Bad Request",
            r#"{"error":{"message":"bad input","type":"invalid_request_error","param":"input","code":"invalid"}}"#,
        ),
        send(&executor, &deployment, &request)
    );
    match result {
        Err(ModerationError::Provider { response }) => {
            assert_eq!(response.status, 400);
            assert!(!response.body.is_empty());
        }
        other => panic!("expected a provider failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_success_body_that_is_not_a_moderation_response_is_a_parse_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (executor, deployment) =
        moderation_executor(&format!("http://{address}/v1"), ", api_key: key");
    let request = ModerationRequest::parse(br#"{"input":"hello"}"#).expect("request");
    let (_, result) = tokio::join!(
        answer(listener, "200 OK", r#"{"id":"modr-1","results":[]}"#),
        send(&executor, &deployment, &request)
    );
    assert!(matches!(
        result,
        Err(ModerationError::Response(
            crate::moderation_response::ModerationResponseError::Decode { .. }
        ))
    ));
}

#[tokio::test]
async fn an_out_of_range_score_is_a_response_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let (executor, deployment) =
        moderation_executor(&format!("http://{address}/v1"), ", api_key: key");
    let request = ModerationRequest::parse(br#"{"input":"hello"}"#).expect("request");
    let out_of_range =
        include_str!("../../fuzz/corpus/moderation_response/out-of-range-score.json");
    let (_, result) = tokio::join!(
        answer(listener, "200 OK", out_of_range),
        send(&executor, &deployment, &request)
    );
    assert!(matches!(
        result,
        Err(ModerationError::Response(
            crate::moderation_response::ModerationResponseError::Decimal { .. }
        ))
    ));
}

#[test]
fn a_deployment_without_a_key_or_openai_support_is_not_served() {
    let (_, unsupported) = moderation_executor_with_model(
        "https://provider.test/v1",
        ", api_key: key",
        "anthropic/claude-3-5-sonnet",
    );
    assert!(matches!(
        super::servable(&unsupported),
        Err(ModerationError::UnsupportedProvider { .. })
    ));
    let (_, missing_key) = moderation_executor("https://provider.test/v1", "");
    assert!(matches!(
        super::servable(&missing_key),
        Err(ModerationError::MissingApiKey)
    ));
}

async fn answer(listener: TcpListener, status: &str, body: &str) -> (String, Vec<u8>) {
    let (mut stream, _) = listener.accept().await.expect("connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).await.expect("request");
        received.extend_from_slice(&chunk[..read]);
        if let Some(index) = received.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let head = String::from_utf8(received[..header_end].to_vec()).expect("head");
    let length = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    while received.len() < header_end + length {
        let read = stream.read(&mut chunk).await.expect("body");
        received.extend_from_slice(&chunk[..read]);
    }
    let body_bytes = received[header_end..header_end + length].to_vec();
    let head_response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head_response.as_bytes())
        .await
        .expect("response head");
    stream
        .write_all(body.as_bytes())
        .await
        .expect("response body");
    (head, body_bytes)
}

fn moderation_executor(base: &str, api_key: &str) -> (ChatExecutor, ChatDeployment) {
    moderation_executor_with_model(base, api_key, "openai/omni-moderation-latest")
}

fn moderation_executor_with_model(
    base: &str,
    api_key: &str,
    model: &str,
) -> (ChatExecutor, ChatDeployment) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&format!(
        "model_list:\n  - model_name: mod\n    litellm_params: {{model: {model}, api_base: {base}{api_key}}}\n"
    ))
    .expect("config");
    let deployment = ChatDeployment::resolve(
        config.model_list.first().expect("deployment"),
        &OpenAiDefaults::default(),
    )
    .expect("deployment");
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let executor = ChatExecutor {
        client: ProxiedClient::outbound(&policy).expect("client"),
        deployments: vec![deployment.clone()],
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(1)),
        clock: fixed_clock,
        next_id: fixed_id,
        mock: MockSettings::default(),
        mock_chat: unused_chat,
        mock_embedding: unused_embedding,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor"),
        bundled_catalog: Arc::default(),
    };
    (executor, deployment)
}

// the executor's mock callbacks belong to other entries; moderation never reaches them
fn unused_chat(
    _plan: &betterlite_config::sections::mock_response::selection::MockPlan<'_>,
    _usage: betterlite_config::sections::mock_response::UsageDefaults,
    _model: &str,
    _id: &str,
    _created: u64,
) -> Result<ChatResponse, betterlite_config::sections::mock_response::failure::MockError> {
    Err(betterlite_config::sections::mock_response::failure::MockError::InternalServerError)
}

fn unused_embedding(
    _response: Option<&betterlite_config::sections::mock_response::MockResponse>,
    _model: &str,
) -> Result<
    crate::embedding::EmbeddingResponse,
    betterlite_config::sections::mock_response::failure::MockError,
> {
    Err(betterlite_config::sections::mock_response::failure::MockError::EmbeddingFailure)
}

fn fixed_clock() -> u64 {
    1_700_000_000
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}
