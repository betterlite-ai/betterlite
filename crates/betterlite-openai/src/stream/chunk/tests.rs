#![cfg(test)]

use super::Chunk;

#[test]
fn stream_maps_model_and_omits_null_finish() {
    let input = br#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}"#;
    let chunk = Chunk::parse(input, "group", false).unwrap().unwrap();
    assert_eq!(
        serde_json::to_string(&chunk).unwrap(),
        r#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"group","choices":[{"index":0,"delta":{"role":"assistant"}}]}"#
    );
}

#[test]
fn stream_maps_usage_and_can_suppress_it() {
    let input = br#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    assert!(Chunk::parse(input, "group", false).unwrap().is_none());
    let chunk = Chunk::parse(input, "group", true).unwrap().unwrap();
    assert_eq!(
        serde_json::to_string(&chunk).unwrap(),
        r#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"group","choices":[{"index":0,"delta":{}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"completion_tokens_details":{"reasoning_tokens":0,"text_tokens":2}}}"#
    );
}

#[test]
fn stream_defaults_tool_arguments() {
    let input = br#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"t","type":"function","function":{"name":"f"}}]},"finish_reason":"tool_calls"}]}"#;
    let chunk = Chunk::parse(input, "", false).unwrap().unwrap();
    assert_eq!(chunk.model, "up");
    assert_eq!(
        serde_json::to_string(&chunk).unwrap(),
        r#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"t","type":"function","function":{"name":"f","arguments":""}}]},"finish_reason":"tool_calls"}]}"#
    );
}

#[test]
fn stream_rejects_invalid_json() {
    Chunk::parse(b"invalid", "group", false).unwrap_err();
}

#[test]
fn stream_suppresses_inline_usage() {
    let input = br#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[{"index":0,"delta":{"content":"hi"}}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
    let chunk = Chunk::parse(input, "group", false).unwrap().unwrap();
    assert!(chunk.usage.is_none());
    assert_eq!(chunk.choices.len(), 1);
}

#[test]
fn stream_preserves_logprobs() {
    let input = br#"{"id":"c","object":"chat.completion.chunk","created":7,"model":"up","choices":[{"index":0,"delta":{"content":"hi"},"logprobs":{"content":[{"token":"hi","logprob":-1,"bytes":[104,105],"top_logprobs":[{"token":"hi","logprob":-1}]}]}}]}"#;
    let chunk = Chunk::parse(input, "", false).unwrap().unwrap();
    assert_eq!(serde_json::to_vec(&chunk).unwrap(), input);
}
