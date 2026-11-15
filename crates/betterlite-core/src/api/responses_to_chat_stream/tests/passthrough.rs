#![cfg(test)]

use serde_json::json;

use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;

use super::support::translated;

#[test]
fn stateless_translation_passes_a_chat_chunk_through() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "id": "cid",
        "created": 5,
        "model": "m",
        "extra_key": 1,
        "choices": [{"index": 1, "delta": {"content": "c"}, "finish_reason": "max_tokens"}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
    }))
    .unwrap();
    let expected = json!({
        "id": "cid",
        "object": "chat.completion.chunk",
        "created": 5,
        "model": "m",
        "extra_key": 1,
        "choices": [{
            "index": 1,
            "delta": {"content": "c", "role": null, "function_call": null, "tool_calls": null, "audio": null},
            "finish_reason": "length",
            "logprobs": null,
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3,
            "prompt_tokens_details": null,
            "completion_tokens_details": null,
        },
        "provider_specific_fields": null,
    });
    assert_eq!(chunk, expected);
}

#[test]
fn stateless_translation_cleans_a_passthrough_delta() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "dropped"},
            "delta": {
                "content": "c",
                "message": "kept",
                "annotations": null,
                "images": [],
                "thinking_blocks": null,
                "reasoning_items": null,
                "reasoning_content": "rc",
                "reasoning": "r",
                "provider_specific_fields": {"x": 1},
            },
        }],
    }))
    .unwrap();
    assert_eq!(
        chunk["choices"][0],
        json!({
            "index": 0,
            "delta": {
                "content": "c",
                "role": null,
                "function_call": null,
                "tool_calls": null,
                "audio": null,
                "reasoning_content": "rc",
                "message": "kept",
                "reasoning": "r",
                "provider_specific_fields": {"x": 1},
            },
            "finish_reason": null,
            "logprobs": null,
        })
    );
}

#[test]
fn passthrough_delta_maps_reasoning_only_and_keeps_thinking_blocks() {
    let mapped = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"reasoning": "only"}}],
    }))
    .unwrap();
    let delta = &mapped["choices"][0]["delta"];
    assert_eq!(delta["reasoning_content"], json!("only"));
    assert!(delta.get("reasoning").is_none());
    let blocks = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"reasoning": null, "thinking_blocks": [{"type": "thinking"}]}}],
    }))
    .unwrap();
    let delta = &blocks["choices"][0]["delta"];
    assert_eq!(delta["thinking_blocks"], json!([{"type": "thinking"}]));
    assert!(delta.get("reasoning").is_none());
}

#[test]
fn passthrough_function_call_requires_its_arguments() {
    let missing = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"function_call": {"name": "f"}}}],
    }))
    .unwrap_err();
    assert!(matches!(
        missing,
        ResponsesChatStreamError::Field { field: "arguments" }
    ));
    let present = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"function_call": {"arguments": "{}"}}}],
    }))
    .unwrap();
    // FunctionCall's name defaults to unset, so an absent name stays out of the dump
    assert_eq!(
        present["choices"][0]["delta"]["function_call"],
        json!({"arguments": "{}"})
    );
    let named = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"function_call": {"name": null, "arguments": "{}"}}}],
    }))
    .unwrap();
    assert_eq!(
        named["choices"][0]["delta"]["function_call"],
        json!({"name": null, "arguments": "{}"})
    );
    let extra = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"function_call": {"arguments": "{}", "provider_specific_fields": {"p": 1}}}}],
    }))
    .unwrap();
    assert_eq!(
        extra["choices"][0]["delta"]["function_call"],
        json!({"arguments": "{}", "provider_specific_fields": {"p": 1}})
    );
}

#[test]
fn passthrough_tool_function_defaults_its_arguments_to_an_empty_string() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {"tool_calls": [{"index": 0, "function": {"name": "f"}}]},
        }],
    }))
    .unwrap();
    assert_eq!(
        chunk["choices"][0]["delta"]["tool_calls"][0]["function"],
        json!({"name": "f", "arguments": ""})
    );
}

#[test]
fn passthrough_tool_function_keeps_only_name_and_arguments() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {"tool_calls": [{
                "index": 0,
                "sibling": 4,
                "function": {"name": "f", "arguments": null, "provider_specific_fields": {"p": 1}},
            }]},
        }],
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["function"], json!({"name": "f", "arguments": ""}));
    assert_eq!(call["sibling"], json!(4));
}

#[test]
fn passthrough_tool_function_stringifies_dict_arguments() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"name": "f", "arguments": {"a": 1}}}]}}],
    }))
    .unwrap();
    assert_eq!(
        chunk["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        json!("{\"a\":1}")
    );
    let parameters = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"name": "f", "parameters": {"b": 2}}}]}}],
    }))
    .unwrap();
    assert_eq!(
        parameters["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        json!("{\"b\":2}")
    );
}

#[test]
fn passthrough_custom_tool_calls_keep_their_shape() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{
            "index": 0,
            "delta": {"tool_calls": [{
                "index": 0,
                "custom": {"name": "x"},
                "extra": 3,
                "function": null,
            }]},
        }],
    }))
    .unwrap();
    assert_eq!(
        chunk["choices"][0]["delta"]["tool_calls"][0],
        json!({"custom": {"name": "x"}, "index": 0, "extra": 3})
    );
    let typed = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "type": "function", "custom": {"name": "x"}}]}}],
    }))
    .unwrap();
    assert_eq!(
        typed["choices"][0]["delta"]["tool_calls"][0],
        json!({"type": "function", "custom": {"name": "x"}, "index": 0})
    );
    let null_type = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "type": null, "custom": {"name": "x"}}]}}],
    }))
    .unwrap();
    assert_eq!(
        null_type["choices"][0]["delta"]["tool_calls"][0],
        json!({"type": null, "custom": {"name": "x"}, "index": 0})
    );
    // the payload is a pydantic model whose name and input are text or null, and its extras pass through
    let nulls = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "custom": {"name": null, "input": null}}]}}],
    }))
    .unwrap();
    assert_eq!(
        nulls["choices"][0]["delta"]["tool_calls"][0]["custom"],
        json!({"name": null, "input": null})
    );
    let bare = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "custom": 5}]}}],
    }))
    .unwrap_err();
    assert!(matches!(
        bare,
        ResponsesChatStreamError::Field { field: "custom" }
    ));
    let named = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "custom": {"name": 5}}]}}],
    }))
    .unwrap_err();
    assert!(matches!(
        named,
        ResponsesChatStreamError::Field {
            field: "custom.name"
        }
    ));
    let listed = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "custom": {"input": ["x"]}}]}}],
    }))
    .unwrap_err();
    assert!(matches!(
        listed,
        ResponsesChatStreamError::Field {
            field: "custom.input"
        }
    ));
}

#[test]
fn passthrough_delta_keeps_declared_extras() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"custom_null": null, "annotations": [], "custom_list": []}}],
    }))
    .unwrap();
    let delta = &chunk["choices"][0]["delta"];
    assert!(delta.get("custom_null").is_some());
    assert_eq!(delta["custom_null"], json!(null));
    assert_eq!(delta["annotations"], json!([]));
    assert_eq!(delta["custom_list"], json!([]));
    let images = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"images": [{"url": "u"}]}}],
    }))
    .unwrap();
    assert_eq!(
        images["choices"][0]["delta"]["images"],
        json!([{"url": "u"}])
    );
    let audio = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"audio": {"id": "a"}}}],
    }))
    .unwrap();
    assert_eq!(audio["choices"][0]["delta"]["audio"], json!({"id": "a"}));
}

#[test]
fn passthrough_finish_reason_needs_a_nonempty_string() {
    let chunk = translated(&json!({
        "object": "chat.completion.chunk",
        "choices": [{"index": 0, "finish_reason": ""}],
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["finish_reason"], json!(null));
}
