use betterlite_models::decisions::Json;

use crate::count_call::counter::{BedrockTokenCounter, TokenCounter as _};

use super::support::{Fixture, Reply, call, environment, fixture, json, user_messages};

pub(crate) async fn native_count_selection() {
    provider_selection();
    absent_and_empty_messages().await;
    null_messages().await;
    message_truthiness().await;
    tools_and_system().await;
    deployment_defaults().await;
    model_forms_resolve().await;
}

fn provider_selection() {
    let built = fixture(Reply::ok("{}"), environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    assert!(counter.should_use_token_counting_api(Some("bedrock")));
    assert!(!counter.should_use_token_counting_api(Some("bedrock_converse")));
    assert!(!counter.should_use_token_counting_api(Some("bedrock_mantle")));
    assert!(!counter.should_use_token_counting_api(Some("Bedrock")));
    assert!(!counter.should_use_token_counting_api(None));
}

async fn absent_messages(built: Fixture, messages: Option<&Json>) {
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let mut request = call(&Json::Null, None);
    request.messages = messages;
    let outcome = counter.count_tokens(&request).await;
    assert!(outcome.is_none());
    assert!(built.transport.requests().is_empty());
}

async fn absent_and_empty_messages() {
    absent_messages(fixture(Reply::ok("{}"), environment(&[])), None).await;
    let empty = json("[]");
    absent_messages(fixture(Reply::ok("{}"), environment(&[])), Some(&empty)).await;
}

async fn null_messages() {
    let null = Json::Null;
    absent_messages(fixture(Reply::ok("{}"), environment(&[])), Some(&null)).await;
}

async fn message_truthiness() {
    let cases = [
        ("0", false),
        ("0.0", false),
        ("1", true),
        ("false", false),
        ("true", true),
        (r#""""#, false),
        (r#""x""#, true),
        ("{}", false),
        (r#"{"a": 1}"#, true),
    ];
    for (text, counted) in cases {
        let messages = json(text);
        let built = fixture(Reply::ok("{}"), environment(&[]));
        let counter = BedrockTokenCounter {
            handler: built.handler,
        };
        let mut request = call(&Json::Null, None);
        request.messages = Some(&messages);
        let outcome = counter.count_tokens(&request).await;
        assert_eq!(outcome.is_some(), counted, "messages {text}");
    }
}

fn sent_body(counter: &BedrockTokenCounter<super::support::Handler>) -> Json {
    let sent = counter.handler.transport.request();
    serde_json::from_slice(&sent.body).unwrap()
}

async fn tools_and_system() {
    let messages = user_messages();
    let tools = json(r#"[{"name":"lookup","input_schema":{"type":"object"}}]"#);
    let system = Json::Text("be nice".to_owned());
    let mut request = call(&messages, None);
    request.tools = Some(&tools);
    request.system = Some(&system);
    let built = fixture(Reply::ok(r#"{"inputTokens": 3}"#), environment(&[]));
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    counter.count_tokens(&request).await.unwrap();
    let body = sent_body(&counter);
    let Json::Object(fields) = &body else {
        panic!("expected an object body");
    };
    let Some(Json::Object(input)) = fields.get("input") else {
        panic!("expected an input object");
    };
    let Some(Json::Object(converse)) = input.get("converse") else {
        panic!("expected a converse input");
    };
    assert_eq!(
        converse.get("system"),
        Some(&json(r#"[{"text": "be nice"}]"#))
    );
    assert!(converse.contains_key("toolConfig"));
}

async fn deployment_defaults() {
    let messages = user_messages();
    let built = fixture(Reply::ok(r#"{"inputTokens": 4}"#), environment(&[]));
    let transport = built.transport.clone();
    let credentials = built.credentials.clone();
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let outcome = counter.count_tokens(&call(&messages, None)).await.unwrap();
    assert_eq!(outcome.total_tokens, 4);
    assert_eq!(
        transport.request().url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(credentials.calls(), 1);
}

async fn model_forms_resolve() {
    let messages = user_messages();
    let built = fixture(Reply::ok(r#"{"inputTokens": 5}"#), environment(&[]));
    let transport = built.transport.clone();
    let counter = BedrockTokenCounter {
        handler: built.handler,
    };
    let mut request = call(&messages, None);
    request.model_to_use = "anthropic.claude-3-5-sonnet-20241022-v2:0:51k";
    counter.count_tokens(&request).await.unwrap();
    assert_eq!(
        transport.request().url,
        "https://bedrock-runtime.us-west-2.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
}
