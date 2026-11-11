use std::time::Duration;

use betterlite_models::decisions::Json;
use http::Method;

use crate::count_call::counter::{BedrockTokenCounter, CountTokensDeployment, TokenCountResponse};

use super::support::{
    Fixture, MODEL, REQUEST_MODEL, Reply, call, deployment, environment, fixture, header, json,
    object, user_messages,
};

const BODY: &str =
    r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#;
const TOKEN_PATH: &str = "/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens";

pub(crate) async fn count_token_http_request() {
    default_endpoint().await;
    api_base_wins().await;
    configured_endpoint().await;
    environment_endpoint().await;
    only_200_succeeds().await;
    missing_input_tokens_counts_zero().await;
}

fn with_key() -> Vec<(&'static str, Json)> {
    vec![("api_key", Json::Text("fixture-bearer".to_owned()))]
}

async fn counted(
    fixture: Fixture,
    messages: &Json,
    params: &CountTokensDeployment,
) -> TokenCountResponse {
    let counter = BedrockTokenCounter {
        handler: fixture.handler,
    };
    counter
        .count_tokens(&call(messages, Some(params)))
        .await
        .unwrap()
}

async fn default_endpoint() {
    let messages = user_messages();
    let params = deployment(&with_key());
    let built = fixture(Reply::ok(r#"{"inputTokens": 42}"#), environment(&[]));
    let transport = built.transport.clone();
    let outcome = counted(built, &messages, &params).await;
    assert_default_request(&transport.request());
    assert_default_outcome(&outcome);
}

fn assert_default_request(sent: &super::support::Sent) {
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url,
        format!("https://bedrock-runtime.us-west-2.amazonaws.com{TOKEN_PATH}")
    );
    assert_eq!(sent.deadline, Duration::from_secs(30));
    assert_eq!(header(sent, "content-type"), Some("application/json"));
    assert_eq!(header(sent, "authorization"), Some("Bearer fixture-bearer"));
    assert_eq!(String::from_utf8(sent.body.clone()).unwrap(), BODY);
}

fn assert_default_outcome(outcome: &TokenCountResponse) {
    assert_eq!(outcome.total_tokens, 42);
    assert_eq!(outcome.request_model, REQUEST_MODEL);
    assert_eq!(outcome.model_used, MODEL);
    assert_eq!(outcome.tokenizer_type, "bedrock_api");
    assert_eq!(
        outcome.original_response,
        Some(object(json(r#"{"input_tokens": 42}"#)))
    );
    assert!(!outcome.error);
    assert_eq!(outcome.error_message, None);
    assert_eq!(outcome.status_code, None);
}

async fn api_base_wins() {
    let messages = user_messages();
    let params = deployment(&[
        ("api_key", Json::Text("fixture-bearer".to_owned())),
        ("api_base", Json::Text("https://api.example".to_owned())),
        (
            "aws_bedrock_runtime_endpoint",
            Json::Text("https://runtime.example".to_owned()),
        ),
    ]);
    let built = fixture(
        Reply::ok(r#"{"inputTokens": 7}"#),
        environment(&[("AWS_BEDROCK_RUNTIME_ENDPOINT", "https://env.example")]),
    );
    let transport = built.transport.clone();
    let outcome = counted(built, &messages, &params).await;
    assert_eq!(
        transport.request().url,
        format!("https://api.example{TOKEN_PATH}")
    );
    assert_eq!(outcome.total_tokens, 7);
}

async fn configured_endpoint() {
    let params = deployment(&[
        ("api_key", Json::Text("fixture-bearer".to_owned())),
        (
            "aws_bedrock_runtime_endpoint",
            Json::Text("https://runtime.example".to_owned()),
        ),
    ]);
    endpoint_url(&params, r#"{"inputTokens": 8}"#, "https://runtime.example").await;
}

async fn environment_endpoint() {
    endpoint_url(
        &deployment(&with_key()),
        r#"{"inputTokens": 9}"#,
        "https://env.example",
    )
    .await;
}

async fn endpoint_url(params: &CountTokensDeployment, reply: &str, expected: &str) {
    let messages = user_messages();
    let built = fixture(
        Reply::ok(reply),
        environment(&[("AWS_BEDROCK_RUNTIME_ENDPOINT", "https://env.example")]),
    );
    let transport = built.transport.clone();
    counted(built, &messages, params).await;
    assert_eq!(transport.request().url, format!("{expected}{TOKEN_PATH}"));
}

async fn only_200_succeeds() {
    let messages = user_messages();
    let params = deployment(&with_key());
    let cases = [
        (200_u16, r#"{"inputTokens": 1}"#, true),
        (201, "answer", false),
        (302, "answer", false),
        (429, "answer", false),
    ];
    for (status, body, succeeds) in cases {
        let built = fixture(Reply::status(status, body), environment(&[]));
        let outcome = counted(built, &messages, &params).await;
        assert_eq!(!outcome.error, succeeds, "status {status}");
        if !succeeds {
            assert_eq!(outcome.status_code, Some(status));
        }
    }
}

async fn missing_input_tokens_counts_zero() {
    let messages = user_messages();
    let params = deployment(&with_key());
    let built = fixture(Reply::ok("{}"), environment(&[]));
    let outcome = counted(built, &messages, &params).await;
    assert_eq!(outcome.total_tokens, 0);
    assert_eq!(
        outcome.original_response,
        Some(object(json(r#"{"input_tokens": 0}"#)))
    );
}
