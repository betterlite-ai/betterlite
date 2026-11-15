use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use crate::stream_chunk_shaping::records::JsonValue;
use crate::streamed_response_assembly::MetadataServices;

use super::support::{
    FixtureResolver, FixtureSelector, RecordingProjection, chunks, priced_logging,
};
use super::{
    Harness, chat, chat_of, cost_stream, failed, hidden_stream, object, ok, plain_stream, value,
};

pub(super) async fn usage_hidden_metadata() {
    usage_reasoning_and_hidden().await;
    hidden_provider_fields().await;
    hidden_provider_fields_error().await;
    prompt_callback_failure().await;
    usage_first_chunk_model().await;
    metadata_after_cost().await;
}

pub(super) async fn usage_reasoning_and_hidden() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hello",
            "reasoning_content": "r"}}]},
        {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
         "_hidden_params": {"provider_specific_fields": {"traffic_type": "on_demand"}}}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.prompt = Some(Box::new(|| Ok(10)));
    harness.counter.text = 1;
    harness.counter.response_text = 2;
    let response = chat_of(&harness).await;
    assert_eq!(harness.counter.seen_completion().as_deref(), Some("Hello"));
    assert_eq!(harness.counter.seen_reasoning().as_deref(), Some("r"));
    assert_eq!(harness.counter.seen_model().as_deref(), Some("gpt-4o"));
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, 12);
    let details = response
        .usage
        .completion_tokens_details
        .as_ref()
        .expect("the complex path counts reasoning tokens");
    assert_eq!(details.reasoning_tokens, Some(1));
    assert_eq!(details.text_tokens, Some(1));
    assert_eq!(
        response.hidden_params.get("provider_specific_fields"),
        Some(&value(r#"{"traffic_type": "on_demand"}"#))
    );
}

const HIDDEN_STREAM: &str = r#"[
    {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
     "choices": [{"index": 0, "delta": {"content": "A"}}],
     "_hidden_params": {"provider_specific_fields": {"a": 1}}},
    {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "_hidden_params": LATER,
     "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}
]"#;

// The nearest later chunk with provider_specific_fields wins and stops the scan, even when its map is empty.
pub(super) async fn hidden_provider_fields() {
    let cases = [
        (
            r#"{"other": "x"}"#,
            r#"{"other": "x", "provider_specific_fields": {"a": 1}}"#,
        ),
        (
            r#"{"provider_specific_fields": {}}"#,
            r#"{"provider_specific_fields": {}}"#,
        ),
        (r#""nope""#, r#"{"provider_specific_fields": {"a": 1}}"#),
    ];
    for (later, expected) in cases {
        let stream = chunks(&HIDDEN_STREAM.replace("LATER", later));
        let response = chat_of(&Harness::new(Some(stream))).await;
        assert_eq!(response.hidden_params, object(expected), "{later}");
    }
}

const NOT_AN_OBJECT: &str = "provider_specific_fields is not a JSON object";

pub(super) async fn hidden_provider_fields_error() {
    let stream = hidden_stream(r#"{"provider_specific_fields": "nope"}"#);
    let (result, _logs) = Harness::new(Some(stream)).run().await;
    assert_eq!(failed(result).to_string(), NOT_AN_OBJECT);
}

pub(super) async fn prompt_callback_failure() {
    let mut harness = Harness::new(Some(plain_stream()));
    harness.messages = true;
    harness.prompt = Some(Box::new(|| Err(CountError::MissingInput)));
    harness.counter.response_text = 1;
    let (result, logs) = harness.run().await;
    let response = chat(ok(result));
    assert_eq!(response.usage.prompt_tokens, 0);
    assert_eq!(response.usage.completion_tokens, 1);
    assert!(
        logs.iter()
            .any(|line| line == "token_counter failed, assuming prompt tokens is 0")
    );
}

pub(super) async fn usage_first_chunk_model() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "azure-model-router",
         "choices": [{"index": 0, "delta": {"role": "assistant", "content": "A"}}]},
        {"model": "gpt-4.1-nano", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.model, "gpt-4.1-nano");
    assert_eq!(
        harness.counter.seen_model().as_deref(),
        Some("azure-model-router")
    );
}

pub(super) async fn metadata_after_cost() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor compiles");
    let resolver = FixtureResolver;
    let selector = FixtureSelector;
    let projection = RecordingProjection::default();
    let services = MetadataServices {
        redactor: &redactor,
        resolver: &resolver,
        selector: &selector,
        projection: Some(&projection),
    };
    let logging = priced_logging("openai", Some(Decimal::new(75, 6)));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    harness.metadata = Some(&services);
    let response = chat_of(&harness).await;
    assert_eq!(projection.seen_cost(), Some(value("0.000075")));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.000075"))
    );
    assert_eq!(
        response.hidden_params.get("projected"),
        Some(&JsonValue::String("OpenAIGPTConfig".to_owned()))
    );
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
}
