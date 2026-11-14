#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::Value;

use super::CompletionResponse;

const ANSWER: &str = r#"{
  "id": "cmpl-1",
  "object": "text_completion",
  "created": 1700000000,
  "model": "gpt-3.5-turbo-instruct",
  "choices": [
    {
      "text": " hello",
      "index": 0,
      "logprobs": {"tokens": [" hello"], "token_logprobs": [-0.1], "top_logprobs": [{" hello": -0.1}], "text_offset": [0]},
      "finish_reason": "stop"
    }
  ],
  "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5},
  "system_fingerprint": "fp-1"
}"#;

#[test]
fn a_completion_body_keeps_its_choices_text_and_logprobs() {
    let response: CompletionResponse = serde_json::from_str(ANSWER).expect("completion body");
    assert_eq!(response.object, "text_completion");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp-1"));
    let choice = response.choices.first().expect("choice");
    assert_eq!(choice.text.as_deref(), Some(" hello"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop"));
    assert!(choice.logprobs.is_some());
    let usage = response.usage.as_ref().expect("usage");
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.total_tokens, 5);
}

// B188's redact_choice blanks a choice's text in the logging payload, so the body the route logs keeps that key
#[test]
fn the_serialized_body_keeps_the_choice_text_where_the_privacy_projection_reads_it() {
    let response: CompletionResponse = serde_json::from_str(ANSWER).expect("completion body");
    let logged = serde_json::to_value(&response).expect("logged body");
    let text = logged
        .pointer("/choices/0/text")
        .and_then(Value::as_str)
        .expect("choice text");
    assert_eq!(text, " hello");
    assert!(
        logged
            .pointer("/choices/0/logprobs/token_logprobs")
            .is_some(),
        "the provider's logprobs stay in the logged body"
    );
}

#[test]
fn unknown_provider_fields_survive_the_round_trip() {
    let body = r#"{"id":"cmpl-2","object":"text_completion","created":1,"model":"m","choices":[],"usage":null,"provider_extra":{"a":1}}"#;
    let response: CompletionResponse = serde_json::from_str(body).expect("completion body");
    let dumped = serde_json::to_value(&response).expect("dump");
    assert_eq!(
        dumped.get("provider_extra"),
        Some(&serde_json::from_str::<Value>(r#"{"a":1}"#).expect("extra"))
    );
}

#[test]
fn a_body_whose_fields_are_missing_takes_the_defaults_litellm_gives_them() {
    let response: CompletionResponse =
        serde_json::from_str(r#"{"object":"text_completion"}"#).expect("empty body");
    assert_eq!(response.id, "");
    assert_eq!(response.created, 0);
    assert_eq!(response.model, None);
    assert!(response.choices.is_empty());
    assert_eq!(response.usage, None);
}

#[rstest]
fn replays_the_text_response_corpus(#[files("fuzz/corpus/text_response/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let _response = serde_json::from_slice::<CompletionResponse>(&bytes);
}
