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
#[case::response(test_snapshot::<ChatResponse>, r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","tool_calls":[{"id":"t","type":"function","function":{"name":"tool","arguments":"private"}}],"audio":{"data":"private"},"annotations":["private"]},"logprobs":{"content":"private"}}]}"#)]
#[case::chunk(test_snapshot::<Chunk>, r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"private sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","refusal":"private","tool_calls":[{"index":0,"function":{"arguments":"private"}}],"function_call":{"arguments":"private"}}}]}"#)]
#[case::embedding(test_snapshot::<EmbeddingResponse>, r#"{"object":"list","model":"m","data":[{"object":"embedding","index":0,"embedding":"private"}],"usage":{"prompt_tokens":1,"total_tokens":1}}"#)]
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
