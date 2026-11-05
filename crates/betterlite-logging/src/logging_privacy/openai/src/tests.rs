#![cfg(test)]

use super::Payload;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde::de::DeserializeOwned;

#[rstest]
#[case::multipart(test_snapshot::<ChatRequest>, r#"{"messages":[{"role":"user","content":[{"type":"text","text":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}]}"#)]
#[case::logprob_credentials(test_logprobs, r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{},"logprobs":{"content":[{"token":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","logprob":0,"bytes":[115],"top_logprobs":[{"token":"sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","logprob":0,"bytes":[115]}]}]}}]}"#)]
#[case::request(test_snapshot::<ChatRequest>, r#"{"messages":[{"role":"user","content":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"metadata":{"input":"campaign","api_key":"private"}}"#)]
#[case::response(test_response, r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","tool_calls":[{"id":"t","type":"function","function":{"name":"tool","arguments":"private"}}],"audio":{"data":"private"},"annotations":["private"]},"logprobs":{"content":"private"}}]}"#)]
#[case::chunk(test_chunk, r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","refusal":"private","tool_calls":[{"index":0,"function":{"arguments":"private"}}],"function_call":{"arguments":"private"}}}]}"#)]
#[case::embedding(test_snapshot::<EmbeddingResponse>, r#"{"object":"list","model":"m","data":[{"object":"embedding","index":0,"embedding":"private"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#)]
#[case::request_credentials(test_credentials::<ChatRequest>, r#"{"model":"$secret","user":"$secret","service_tier":"$secret","reasoning_effort":"$secret","stop":["$secret"],"tool_choice":{"type":"$secret","function":{"name":"$secret"}},"tools":[{"type":"$secret","function":{"name":"$secret","description":"$secret","parameters":{"description":"$secret"}},"custom":"$secret"}],"response_format":{"type":"$secret","json_schema":{"description":"$secret"}},"messages":[{"role":"assistant","name":"$secret","tool_call_id":"$secret","tool_calls":[{"id":"$secret","type":"$secret","function":{"name":"$secret","arguments":"$secret"}}],"content":[{"type":"$secret","text":"$secret"}]}]}"#)]
#[case::request_modes(test_credentials::<ChatRequest>, r#"{"messages":[],"stop":"$secret","tool_choice":"$secret"}"#)]
#[case::response_credentials(test_credentials::<ChatResponse>, r#"{"id":"$secret","object":"$secret","model":"$secret","created":1,"system_fingerprint":"$secret","service_tier":"$secret","choices":[{"index":0,"finish_reason":"$secret","message":{"role":"$secret","content":"$secret","tool_calls":[{"id":"$secret","type":"$secret","function":{"name":"$secret","arguments":"$secret"}}]}}]}"#)]
#[case::chunk_credentials(test_credentials::<Chunk>, r#"{"id":"$secret","object":"$secret","model":"$secret","created":1,"system_fingerprint":"$secret","service_tier":"$secret","choices":[{"index":0,"finish_reason":"$secret","delta":{"role":"$secret","content":"$secret","refusal":"$secret","tool_calls":[{"index":0,"id":"$secret","type":"$secret","function":{"name":"$secret","arguments":"$secret"}}],"function_call":{"name":"$secret","arguments":"$secret"}}}]}"#)]
#[case::embedding_credentials(test_credentials::<EmbeddingResponse>, r#"{"object":"$secret","model":"$secret","data":[{"object":"$secret","index":0,"embedding":"$secret"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#)]
fn logging_privacy_tests_behavior(#[case] check: fn(&str), #[case] input: &str) {
    check(input);
}

fn test_snapshot<P>(input: &str)
where
    P: Payload + DeserializeOwned,
{
    let source: P = serde_json::from_str(input).unwrap();
    let original = serde_json::to_string(&source).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let private = source.snapshot(true, &redactor).unwrap();
    let public = source.snapshot(false, &redactor).unwrap();
    assert!(!serde_json::to_string(&private).unwrap().contains("private"));
    assert!(serde_json::to_string(&public).unwrap().contains("private"));
    assert!(!serde_json::to_string(&public).unwrap().contains("sk-aaaa"));
    assert_eq!(serde_json::to_string(&source).unwrap(), original);
}

fn test_logprobs(input: &str) {
    let source: Chunk = serde_json::from_str(input).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let safe = source.snapshot(false, &redactor).unwrap();
    let token = &safe.choices[0]
        .logprobs
        .as_ref()
        .unwrap()
        .content
        .as_ref()
        .unwrap()[0];
    assert!(token.bytes.is_none());
    assert!(token.top_logprobs[0].bytes.is_none());
    assert!(!token.token.contains("sk-aaaa"));
    assert!(!token.top_logprobs[0].token.contains("sk-aaaa"));
}

fn test_response(input: &str) {
    test_snapshot::<ChatResponse>(input);
    let source: ChatResponse = serde_json::from_str(input).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let private = source.snapshot(true, &redactor).unwrap();
    let public = source.snapshot(false, &redactor).unwrap();
    assert_eq!(
        private.choices[0].message.content.as_deref(),
        Some("redacted-by-litellm")
    );
    assert_eq!(
        private.choices[0].message.tool_calls.as_ref().unwrap()[0]
            .function
            .arguments,
        "redacted-by-litellm"
    );
    assert_eq!(
        public.choices[0].message.tool_calls.as_ref().unwrap()[0]
            .function
            .arguments,
        "private"
    );
    assert!(private.choices[0].message.audio.is_none());
    assert!(private.choices[0].message.annotations.is_none());
    assert!(private.choices[0].logprobs.is_none());
}

fn test_chunk(input: &str) {
    test_snapshot::<Chunk>(input);
    let source: Chunk = serde_json::from_str(input).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let private = source.snapshot(true, &redactor).unwrap();
    let public = source.snapshot(false, &redactor).unwrap();
    let private_delta = &private.choices[0].delta;
    let public_delta = &public.choices[0].delta;
    assert_eq!(
        private_delta.content.as_deref(),
        Some("redacted-by-litellm")
    );
    assert_eq!(
        private_delta.function_call.as_ref().unwrap().arguments,
        "redacted-by-litellm"
    );
    assert_eq!(
        private_delta.tool_calls.as_ref().unwrap()[0]
            .function
            .as_ref()
            .unwrap()
            .arguments,
        "redacted-by-litellm"
    );
    assert_eq!(
        public_delta.function_call.as_ref().unwrap().arguments,
        "private"
    );
    assert_eq!(
        public_delta.tool_calls.as_ref().unwrap()[0]
            .function
            .as_ref()
            .unwrap()
            .arguments,
        "private"
    );
}

fn test_credentials<P>(input: &str)
where
    P: Payload + DeserializeOwned,
{
    let fixture = input.replace("$secret", "sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let source: P = serde_json::from_str(&fixture).unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let public = source.snapshot(false, &redactor).unwrap();
    let private = source.snapshot(true, &redactor).unwrap();
    assert!(!serde_json::to_string(&public).unwrap().contains("sk-aaaa"));
    assert!(!serde_json::to_string(&private).unwrap().contains("sk-aaaa"));
    assert!(serde_json::to_string(&source).unwrap().contains("sk-aaaa"));
}
