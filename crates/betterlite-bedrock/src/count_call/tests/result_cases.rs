use betterlite_models::decisions::Json;

use crate::count_call::counter::BedrockTokenCounter;

use super::logs::{Kind, capture};
use super::support::{
    FixedHandler, Reply, call, deployment, environment, fixture, json, object, user_messages,
};

pub(crate) async fn count_result_or_none() {
    none_result_returns_none().await;
    zero_result_is_a_count().await;
    missing_key_defaults_to_zero().await;
    null_input_tokens_fails_validation().await;
    text_input_tokens_fails_validation().await;
    api_failure_is_not_none().await;
}

async fn none_result_returns_none() {
    let messages = user_messages();
    let counter = BedrockTokenCounter {
        handler: FixedHandler { result: None },
    };
    assert!(counter.count_tokens(&call(&messages, None)).await.is_none());
}

async fn zero_result_is_a_count() {
    fixed_zero(r#"{"input_tokens": 0}"#).await;
}

async fn missing_key_defaults_to_zero() {
    fixed_zero(r#"{"other": true}"#).await;
}

async fn fixed_zero(reply: &str) {
    let messages = user_messages();
    let result = object(json(reply));
    let expected = result.clone();
    let counter = BedrockTokenCounter {
        handler: FixedHandler {
            result: Some(result),
        },
    };
    let outcome = counter.count_tokens(&call(&messages, None)).await.unwrap();
    assert_eq!(outcome.total_tokens, 0);
    assert!(!outcome.error);
    assert_eq!(outcome.original_response, Some(expected));
}

async fn validation_failure(result: Json) {
    let messages = user_messages();
    let (logs, guard) = capture();
    let counter = BedrockTokenCounter {
        handler: FixedHandler {
            result: Some(object(result)),
        },
    };
    let outcome = counter.count_tokens(&call(&messages, None)).await.unwrap();
    drop(guard);
    assert!(outcome.error);
    assert_eq!(outcome.total_tokens, 0);
    assert_eq!(outcome.status_code, Some(500));
    assert_eq!(outcome.original_response, None);
    assert_eq!(
        outcome.error_message.as_deref(),
        Some("input_tokens is not an integer")
    );
    assert_eq!(
        logs.subjects(Kind::Warn),
        ["Error calling Bedrock CountTokens API"]
    );
}

async fn null_input_tokens_fails_validation() {
    validation_failure(json(r#"{"input_tokens": null}"#)).await;
}

async fn text_input_tokens_fails_validation() {
    validation_failure(json(r#"{"input_tokens": "5"}"#)).await;
}

async fn api_failure_is_not_none() {
    let messages = user_messages();
    let params = deployment(&[("api_key", Json::Text("fixture-bearer".to_owned()))]);
    let built = fixture(Reply::status(401, "denied"), environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(401));
}
