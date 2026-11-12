use std::time::Duration;

use betterlite_models::decisions::Json;

use crate::count_call::counter::BedrockTokenCounter;

use super::support::{Reply, call, deployment, environment, fixture, user_messages};

pub(crate) async fn count_timeout() {
    literal_thirty_seconds().await;
    timeout_is_not_a_408().await;
}

async fn literal_thirty_seconds() {
    let messages = user_messages();
    let params = deployment(&[
        ("api_key", Json::Text("fixture-bearer".to_owned())),
        ("timeout", Json::Number(serde_json::Number::from(1_u8))),
        (
            "request_timeout",
            Json::Number(serde_json::Number::from(2_u8)),
        ),
    ]);
    let built = fixture(Reply::ok(r#"{"inputTokens": 1}"#), environment(&[]));
    let transport = built.transport.clone();
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    assert_eq!(transport.request().deadline, Duration::from_secs(30));
}

async fn timeout_is_not_a_408() {
    let messages = user_messages();
    let params = deployment(&[("api_key", Json::Text("fixture-bearer".to_owned()))]);
    let built = fixture(Reply::Failure, environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter
        .count_tokens(&call(&messages, Some(&params)))
        .await
        .unwrap();
    assert!(outcome.error);
    assert_eq!(outcome.status_code, Some(500));
    assert_ne!(outcome.status_code, Some(408));
    assert!(
        outcome
            .error_message
            .as_deref()
            .unwrap()
            .starts_with("CountTokens processing error: ")
    );
}
