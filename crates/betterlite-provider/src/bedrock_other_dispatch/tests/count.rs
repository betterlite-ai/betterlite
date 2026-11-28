use std::time::SystemTime;

use betterlite_bedrock::count_call::counter::{
    CountTokensCall, CountTokensDeployment, TokenCountResponse,
};
use betterlite_bedrock::count_call::handler::{BedrockCountTokensHandler, CountTokensEnvironment};
use betterlite_models::decisions::Json;

use super::fixture::{Fixture, Handler, object, params, run, value};
use crate::bedrock_other_dispatch::call::{BedrockOtherOperation, CountOperation};
use crate::bedrock_other_dispatch::test_support::client::Reply;
use crate::bedrock_other_dispatch::test_support::{NOW_SECONDS, credentials, instant};

const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const MODEL: &str = "amazon.titan-text-express-v1";
const DEPLOYMENT: &str = r#"{"aws_region_name": "us-east-1"}"#;
const BODY: &str =
    r#"{"input": {"converse": {"messages": [{"role": "user", "content": [{"text": "hi"}]}]}}}"#;

fn scripted(replies: &[Reply]) -> Fixture {
    Fixture::new(replies, credentials(ACCESS_KEY, SECRET_KEY))
}

fn handler(fixture: &Fixture) -> Handler {
    BedrockCountTokensHandler {
        transport: fixture.client(),
        credentials: fixture.source().clone(),
        environment: CountTokensEnvironment::default(),
    }
}

fn now() -> SystemTime {
    instant(NOW_SECONDS).expect("the fixture instant")
}

fn operation<'call>(
    messages: Option<&'call Json>,
    deployment: Option<&'call CountTokensDeployment>,
    handler: Option<Handler>,
) -> BedrockOtherOperation<'call, 'call, 'call, Handler, super::fixture::Binding> {
    BedrockOtherOperation::Count(CountOperation {
        call: CountTokensCall {
            model_to_use: MODEL,
            messages,
            deployment,
            request_model: MODEL,
            tools: None,
            system: None,
            now: now(),
        },
        handler,
    })
}

fn deployment() -> CountTokensDeployment {
    CountTokensDeployment {
        litellm_params: params(DEPLOYMENT),
    }
}

fn success_fields(response: &TokenCountResponse) -> (&str, &str, &str) {
    (
        response.request_model.as_str(),
        response.model_used.as_str(),
        response.tokenizer_type.as_str(),
    )
}

// The resolved model reaches the CountTokens endpoint and its reply becomes the caller's token count
async fn counted_tokens() {
    let fixture = scripted(&[Reply::status(200, r#"{"inputTokens": 42}"#)]);
    let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
    let deployment = deployment();
    let outcome = run(
        &fixture,
        operation(Some(&messages), Some(&deployment), Some(handler(&fixture))),
    )
    .await;
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.posted().method, "POST");
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-text-express-v1/count-tokens"
    );
    assert_eq!(outcome.body(0), value(BODY));
    let response = outcome.count().expect("the handler answers a count");
    assert_eq!(response.total_tokens, 42);
    assert_eq!(success_fields(response), (MODEL, MODEL, "bedrock_api"));
    assert!(!response.error);
    assert_eq!(response.error_message, None);
    assert_eq!(response.status_code, None);
    assert_eq!(
        response.original_response,
        Some(object(&value(r#"{"input_tokens": 42}"#)).clone())
    );
}

// No messages, an empty message list, and no handler are all the same answer: no count at all
async fn absent_calls() {
    let fixture = scripted(&[]);
    let deployment = deployment();
    for messages in [None, Some(value("[]")), Some(value("null"))] {
        let outcome = run(
            &fixture,
            operation(
                messages.as_ref(),
                Some(&deployment),
                Some(handler(&fixture)),
            ),
        )
        .await;
        assert!(outcome.count().is_none(), "messages {messages:?}");
        assert!(outcome.requests.is_empty());
    }
    let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
    let outcome = run(
        &fixture,
        operation(Some(&messages), Some(&deployment), None),
    )
    .await;
    assert!(outcome.count().is_none());
    assert!(outcome.requests.is_empty());
}

// A provider error becomes a failed count record with its status and the provider's message
async fn provider_refusal() {
    let fixture = scripted(&[Reply::status(400, "bad request")]);
    let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
    let count_deployment = deployment();
    let outcome = run(
        &fixture,
        operation(
            Some(&messages),
            Some(&count_deployment),
            Some(handler(&fixture)),
        ),
    )
    .await;
    let refusal = outcome
        .count()
        .expect("a failed call still answers a record");
    assert!(refusal.error, "the record reports the failure");
    assert_eq!(refusal.total_tokens, 0);
    assert!(refusal.original_response.is_none());
    assert_eq!(refusal.status_code, Some(400));
    assert_eq!(refusal.error_message.as_deref(), Some("bad request"));
    assert_eq!(outcome.requests.len(), 1);
}

// The deployment's own static keys sign the count request, not the identity the source was built with
async fn deployment_keys() {
    let fixture = scripted(&[Reply::status(200, r#"{"inputTokens": 1}"#)]);
    let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
    let own_keys = CountTokensDeployment {
        litellm_params: params(
            r#"{"aws_access_key_id": "AKIAI44QH8DHBEXAMPLE", "aws_secret_access_key": "je7MtGbClwBF/2Zp9Utk/h3yCo8nvbEXAMPLEKEY", "aws_region_name": "us-east-1"}"#,
        ),
    };
    let outcome = run(
        &fixture,
        operation(Some(&messages), Some(&own_keys), Some(handler(&fixture))),
    )
    .await;
    let authorization = outcome
        .authorization()
        .expect("the deployment's keys sign the count");
    assert!(authorization.contains("AKIAI44QH8DHBEXAMPLE"));
    assert!(!authorization.contains(ACCESS_KEY));
}

// A transport failure and an unparsable reply are reported as 500 records, never counted locally
async fn unreadable_replies() {
    for reply in [Reply::Failed, Reply::status(200, "not json")] {
        let fixture = scripted(&[reply]);
        let messages = value(r#"[{"role": "user", "content": "hi"}]"#);
        let count_deployment = deployment();
        let outcome = run(
            &fixture,
            operation(
                Some(&messages),
                Some(&count_deployment),
                Some(handler(&fixture)),
            ),
        )
        .await;
        let response = outcome
            .count()
            .expect("a failed call still answers a record");
        assert!(response.error);
        assert_eq!(response.status_code, Some(500));
        assert!(
            response
                .error_message
                .as_deref()
                .is_some_and(|message| message.starts_with("CountTokens processing error: "))
        );
    }
}

pub(super) async fn case() {
    counted_tokens().await;
    absent_calls().await;
    provider_refusal().await;
    deployment_keys().await;
    unreadable_replies().await;
}
