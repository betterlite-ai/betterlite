#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::Value;

use super::{CompletionPrompt, CompletionRequest, CompletionRequestError};

fn parsed(body: &str) -> CompletionRequest {
    CompletionRequest::parse(body.as_bytes()).expect("completion request")
}

#[rstest]
#[case::string(r#""hello""#, r#"[{"role":"user","content":"hello"}]"#)]
#[case::strings(
    r#"["a","b"]"#,
    r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#
)]
#[case::tokens(r#"[1,2]"#, r#"[{"role":"user","content":[1,2]}]"#)]
#[case::token_lists(r#"[[1,2],[3]]"#, r#"[{"role":"user","content":[[1,2],[3]]}]"#)]
#[case::empty_string(r#""""#, r#"[{"role":"user","content":""}]"#)]
fn prompt_shapes_become_messages(#[case] prompt: &str, #[case] expected: &str) {
    let body = format!(r#"{{"model":"m","prompt":{prompt}}}"#);
    let request = parsed(&body);
    let messages = request.prompt.messages().expect("messages");
    let expected: Value = serde_json::from_str(expected).expect("expected messages");
    assert_eq!(Value::Array(messages), expected);
}

#[rstest]
#[case::empty_list("[]")]
#[case::mixed_string_and_number(r#"["a",1]"#)]
#[case::nested_non_tokens(r#"[["a"]]"#)]
#[case::number(r#"5"#)]
#[case::null("null")]
#[case::object(r#"{"a":1}"#)]
fn unmapped_prompts_keep_their_shape_and_build_no_messages(#[case] prompt: &str) {
    let body = format!(r#"{{"model":"m","prompt":{prompt}}}"#);
    let request = parsed(&body);
    assert!(matches!(&request.prompt, CompletionPrompt::Unmapped(_)));
    assert!(request.prompt.messages().is_none());
}

#[test]
fn a_missing_prompt_is_a_body_error() {
    let error = CompletionRequest::parse(br#"{"model":"m"}"#).expect_err("missing prompt");
    assert!(matches!(error, CompletionRequestError::Body { .. }));
}

#[test]
fn token_prompts_serialize_ids_as_numbers_in_their_nested_shape() {
    let request = parsed(r#"{"model":"m","prompt":[[1,2],[3]]}"#);
    assert_eq!(
        Value::from(request.prompt.clone()),
        serde_json::from_str::<Value>("[[1,2],[3]]").expect("token lists")
    );
    assert_eq!(
        parsed(r#"{"model":"m","prompt":"x"}"#).prompt,
        CompletionPrompt::Text(String::from("x"))
    );
}

#[test]
fn the_model_is_optional_and_kept_apart_from_the_provider_params() {
    let request = parsed(r#"{"model":"gpt-3.5-turbo-instruct","prompt":"Once upon a time"}"#);
    assert_eq!(request.model.as_deref(), Some("gpt-3.5-turbo-instruct"));
    let params = request.optional_params().expect("optional params");
    assert!(!params.contains_key("model"));
    assert!(!params.contains_key("prompt"));
}

#[rstest]
#[case::api_base("api_base")]
#[case::api_key("api_key")]
#[case::metadata("metadata")]
#[case::mock_response("mock_response")]
#[case::num_retries("num_retries")]
#[case::timeout("timeout")]
fn controls_never_reach_the_provider_params(#[case] control: &str) {
    let request = parsed(
        r#"{"model":"m","prompt":"x","api_base":"http://evil","api_key":"sk-1","metadata":{"a":1},"mock_response":"hi","num_retries":2,"timeout":1,"max_retries":4}"#,
    );
    let params = request.optional_params().expect("optional params");
    assert!(!params.contains_key(control), "{control} leaked");
    assert_eq!(params.get("max_retries"), Some(&Value::from(4)));
}

#[test]
fn the_supported_parameters_keep_their_exact_json_values() {
    let request = parsed(
        r#"{"model":"m","prompt":"x","temperature":0.7,"top_p":0.9,"n":2,"logprobs":3,"top_logprobs":1,"max_tokens":16,"stop":["a","b"],"seed":42,"logit_bias":{"50256":-100},"user":"u","presence_penalty":0.1,"frequency_penalty":0.2,"extra_headers":{"x-caller":"yes"}}"#,
    );
    let params = request.optional_params().expect("optional params");
    let text = serde_json::to_string(&Value::Object(params)).expect("params");
    assert!(text.contains("\"temperature\":0.7"), "{text}");
    assert!(text.contains("\"top_p\":0.9"), "{text}");
    assert!(text.contains("\"presence_penalty\":0.1"), "{text}");
    assert!(text.contains("\"frequency_penalty\":0.2"), "{text}");
    assert!(text.contains("\"logit_bias\":{\"50256\":-100}"), "{text}");
    assert!(text.contains("\"logprobs\":3"), "{text}");
    assert!(text.contains("\"top_logprobs\":1"), "{text}");
    assert!(text.contains("\"stop\":[\"a\",\"b\"]"), "{text}");
    assert!(
        text.contains("\"extra_headers\":{\"x-caller\":\"yes\"}"),
        "{text}"
    );
}

#[rstest]
#[case::unknown_parameter(r#"{"model":"m","prompt":"x","top_k":5}"#)]
#[case::best_of_is_not_a_text_parameter(r#"{"model":"m","prompt":"x","best_of":2}"#)]
#[case::echo_is_not_a_text_parameter(r#"{"model":"m","prompt":"x","echo":true}"#)]
fn unsupported_parameters_are_refused_by_name(#[case] body: &str) {
    let error = CompletionRequest::parse(body.as_bytes()).expect_err("unsupported parameter");
    let CompletionRequestError::Unsupported { name } = &error else {
        panic!("the refusal must name the parameter: {error}");
    };
    assert!(!name.is_empty());
}

#[test]
fn stream_is_read_but_kept_out_of_the_provider_params() {
    let request = parsed(r#"{"model":"m","prompt":"x","stream":true}"#);
    assert_eq!(request.stream, Some(true));
    let params = request.optional_params().expect("optional params");
    assert!(!params.contains_key("stream"));
}

#[rstest]
fn replays_the_text_request_corpus(#[files("fuzz/corpus/text_request/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    if let Ok(request) = CompletionRequest::parse(&bytes) {
        let _messages = request.prompt.messages();
        let _params = request.optional_params();
    }
}
