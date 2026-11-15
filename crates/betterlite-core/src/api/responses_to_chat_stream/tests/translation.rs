#![cfg(test)]

use serde_json::json;

use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;
use crate::api::responses_to_chat_stream::translate::{ToolCallIndices, translate_event};

use super::support::{
    GENERATED_ID, choice, chunk_of, empty_delta, event, factory, rendered, stream, translated,
};

#[test]
fn stateless_translation_rejects_empty_and_non_object_chunks() {
    let empty = translate_event(&json!({}), None, factory()).unwrap_err();
    assert_eq!(empty.to_string(), "Chat provider: Empty parsed_chunk");
    let list = translate_event(&json!([1]), None, factory()).unwrap_err();
    assert_eq!(
        list.to_string(),
        "Chat provider: Invalid chunk type <class 'list'>"
    );
    let text = translate_event(&json!("x"), None, factory()).unwrap_err();
    assert_eq!(
        text.to_string(),
        "Chat provider: Invalid chunk type <class 'str'>"
    );
    // the errors carry no HTTP fields, so a caller answers with LiteLLM's plain ValueError
    assert!(matches!(
        list,
        ResponsesChatStreamError::InvalidChunkType { kind: "list" }
    ));
}

#[test]
fn a_typeless_payload_without_choices_is_not_a_chat_chunk() {
    let chunk = translated(&json!({"model": "m"})).unwrap();
    assert_eq!(chunk["id"], json!(GENERATED_ID));
    assert!(chunk.get("model").is_none());
}

#[rstest::rstest]
#[case::integer("5", "int")]
#[case::negative("-1", "int")]
#[case::float("1.5", "float")]
#[case::boolean("true", "bool")]
fn stateless_translation_names_the_rejected_chunk_type(#[case] source: &str, #[case] kind: &str) {
    let failure = translate_event(&event(source), None, factory()).unwrap_err();
    assert_eq!(
        failure.to_string(),
        format!("Chat provider: Invalid chunk type <class '{kind}'>")
    );
}

#[rstest::rstest]
#[case::zero("0")]
#[case::null("null")]
#[case::empty_list("[]")]
#[case::empty_text("\"\"")]
fn stateless_translation_rejects_falsy_chunks(#[case] falsy: &str) {
    let failure = translate_event(&event(falsy), None, factory()).unwrap_err();
    assert_eq!(failure.to_string(), "Chat provider: Empty parsed_chunk");
}

#[test]
fn stateless_translation_passes_a_missing_type_with_choices_through() {
    let chunk =
        translated(&json!({"choices": [{"index": 0, "delta": {"content": "hi"}}]})).unwrap();
    assert_eq!(chunk["id"], json!(GENERATED_ID));
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!("hi"));
    // an empty choices list is not a chat chunk, so the event falls to the unhandled empty chunk
    let bare = translated(&json!({"choices": []})).unwrap();
    assert_eq!(
        bare,
        chunk_of(
            GENERATED_ID,
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_maps_an_unhandled_type_to_an_empty_chunk() {
    let chunk = translated(&json!({"type": "response.failed"})).unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            GENERATED_ID,
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_creates_an_id_from_the_created_event() {
    let chunk = translated(&json!({
        "type": "response.created",
        "response": {"id": "resp_a"},
    }))
    .unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            "resp_a",
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_generates_an_id_without_a_created_response() {
    let ids = [
        json!({"type": "response.created"}),
        json!({"type": "response.created", "response": null}),
        json!({"type": "response.created", "response": {}}),
        json!({"type": "response.created", "response": {"id": null}}),
    ]
    .map(|source| translated(&source).unwrap()["id"].clone());
    assert_eq!(ids, vec![json!(GENERATED_ID); 4]);
}

#[test]
fn stateless_translation_carries_tool_calls_on_a_new_item() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "output_index": 2,
        "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"},
    }))
    .unwrap();
    // call_1 is Bedrock Mantle's index-form call_id, so B917 answers with the item id instead
    let expected_call = json!({
        "id": "fc_1",
        "index": 2,
        "type": "function",
        "function": {"name": "f", "arguments": "{}"},
    });
    let delta = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": [expected_call],
        "audio": null,
    });
    assert_eq!(
        chunk,
        chunk_of(GENERATED_ID, json!([choice(delta, json!(null), json!(0))]))
    );
}

#[test]
fn stateless_translation_defaults_a_missing_output_index_to_zero() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "item": {"type": "custom_tool_call", "call_id": "c1", "input": "raw"},
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["index"], json!(0));
    assert_eq!(call["id"], json!("c1"));
    assert_eq!(call["function"]["name"], json!("custom_tool"));
    assert_eq!(call["function"]["arguments"], json!("raw"));
}

#[test]
fn stateless_translation_turns_an_empty_converted_name_into_null() {
    let chunk = translated(&json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {"type": "function_call", "call_id": "c", "name": "", "arguments": ""},
        "arguments": "top",
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], json!(null));
    assert_eq!(call["function"]["arguments"], json!("top"));
}

#[test]
fn stateless_translation_rejects_an_empty_argument_delta() {
    let failure = translated(&json!({
        "type": "response.function_call_arguments.delta",
        "delta": "",
    }))
    .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Chat provider: Invalid function argument delta {'type': 'response.function_call_arguments.delta', 'delta': ''}"
    );
    let missing = translated(&json!({
        "type": "response.custom_tool_call_input.delta",
        "delta": null,
    }))
    .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "Chat provider: Invalid function argument delta {'type': 'response.custom_tool_call_input.delta', 'delta': None}"
    );
}

#[test]
fn stateless_translation_carries_an_argument_delta_at_the_raw_index() {
    let chunk = translated(&json!({
        "type": "response.custom_tool_call_input.delta",
        "output_index": 5,
        "delta": "x",
    }))
    .unwrap();
    let call = &chunk["choices"][0]["delta"]["tool_calls"][0];
    assert_eq!(
        call,
        &json!({"id": null, "index": 5, "type": "function", "function": {"name": null, "arguments": "x"}})
    );
}

#[test]
fn stateless_translation_rejects_a_missing_text_delta() {
    let failure = translated(&json!({"type": "response.output_text.delta"})).unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Chat provider: Invalid text delta {'type': 'response.output_text.delta'}"
    );
}

#[test]
fn stateless_translation_keeps_an_empty_text_delta() {
    let chunk = translated(&json!({"type": "response.output_text.delta", "delta": ""})).unwrap();
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!(""));
    let text = translated(&json!({"type": "response.output_text.delta", "delta": "hi"})).unwrap();
    assert_eq!(text["choices"][0]["delta"]["content"], json!("hi"));
}

#[test]
fn stateless_translation_uses_the_summary_index_for_reasoning() {
    let chunk = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "summary_index": 3,
        "delta": "x",
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["index"], json!(3));
    assert_eq!(
        chunk["choices"][0]["delta"]["reasoning_content"],
        json!("x")
    );
    let absent = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "delta": "x",
    }))
    .unwrap();
    assert_eq!(absent["choices"][0]["index"], json!(null));
}

#[test]
fn stateless_translation_makes_an_empty_reasoning_delta_an_unhandled_event() {
    let chunk = translated(&json!({
        "type": "response.reasoning_summary_text.delta",
        "summary_index": 1,
        "delta": "",
    }))
    .unwrap();
    assert_eq!(
        chunk,
        chunk_of(
            GENERATED_ID,
            json!([choice(empty_delta(), json!(null), json!(0))])
        )
    );
}

#[test]
fn stateless_translation_done_item_is_not_terminal() {
    let item = json!({"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "f", "arguments": "{}"});
    let complete = translated(&json!({
        "type": "response.output_item.done",
        "output_index": 2,
        "item": item,
    }))
    .unwrap();
    let call = json!({
        "id": "fc_1",
        "index": 2,
        "type": "function",
        "function": {"name": "f", "arguments": "{}"},
    });
    let delta = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": [call],
        "audio": null,
    });
    assert_eq!(
        complete,
        chunk_of(GENERATED_ID, json!([choice(delta, json!(null), json!(0))]))
    );

    // the stateful stream already sent the call, so its done event carries an empty delta and never a finish reason
    let mut iterator = stream();
    let stateful = iterator
        .chunk_parser(&json!({
            "type": "response.output_item.done",
            "output_index": 2,
            "item": item,
        }))
        .unwrap();
    let empty = json!({
        "content": null,
        "role": null,
        "function_call": null,
        "tool_calls": null,
        "audio": null,
    });
    assert_eq!(
        rendered(&stateful),
        chunk_of(GENERATED_ID, json!([choice(empty, json!(null), json!(0))]))
    );
}

#[test]
fn stateless_translation_done_message_gives_empty_content() {
    let chunk = translated(&json!({
        "type": "response.output_item.done",
        "item": {"type": "message"},
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["delta"]["content"], json!(""));
    assert_eq!(chunk["choices"][0]["finish_reason"], json!(null));
}

#[test]
fn stateless_translation_sequential_indices_need_a_map() {
    let added = |index: i64| {
        json!({
            "type": "response.output_item.added",
            "output_index": index,
            "item": {"type": "function_call", "id": "f", "call_id": "c", "name": "n", "arguments": "{}"},
        })
    };
    let mut iterator = stream();
    let indices = [2_i64, 5, 2].map(|index| {
        let chunk = iterator.chunk_parser(&added(index)).unwrap();
        chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0].index
    });
    assert_eq!(indices, [Some(0), Some(1), Some(0)]);
    let raw = [2_i64, 5, 2].map(|index| {
        translated(&added(index)).unwrap()["choices"][0]["delta"]["tool_calls"][0]["index"].clone()
    });
    assert_eq!(raw, [json!(2), json!(5), json!(2)]);
}

#[test]
fn stateless_translation_accepts_a_shared_tool_call_map() {
    let mut map = ToolCallIndices::new();
    let added = |index: i64| {
        json!({
            "type": "response.output_item.added",
            "output_index": index,
            "item": {"type": "function_call", "id": "f", "call_id": "c", "name": "n", "arguments": "{}"},
        })
    };
    let first = translate_event(&added(7), Some(&mut map), factory())
        .map(|chunk| rendered(&chunk))
        .unwrap();
    assert_eq!(
        first["choices"][0]["delta"]["tool_calls"][0]["index"],
        json!(0)
    );
    assert_eq!(map.get(&7), Some(&0));
}

#[test]
fn stateless_translation_finishes_on_completed_output() {
    let plain = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "message"}]},
    }))
    .unwrap();
    assert_eq!(plain["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(plain["usage"], json!(null));
    assert_eq!(plain["provider_specific_fields"], json!(null));

    let calls = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "custom_tool_call"}]},
    }))
    .unwrap();
    assert_eq!(calls["choices"][0]["finish_reason"], json!("tool_calls"));

    let patch = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "output": [{"type": "apply_patch_call"}]},
    }))
    .unwrap();
    assert_eq!(patch["choices"][0]["finish_reason"], json!("stop"));
}

#[test]
fn terminal_usage_needs_a_truthy_payload() {
    let usages = [json!({}), json!(0), json!(""), json!([]), json!(false)].map(|usage| {
        translated(&json!({"type": "response.completed", "response": {"usage": usage}})).unwrap()
            ["usage"]
            .clone()
    });
    assert_eq!(usages, vec![json!(null); 5]);
}

#[test]
fn stateless_translation_finishes_an_incomplete_response_by_its_reason() {
    let filtered = translated(&json!({
        "type": "response.incomplete",
        "response": {"incomplete_details": {"reason": "content_filter"}},
    }))
    .unwrap();
    assert_eq!(
        filtered["choices"][0]["finish_reason"],
        json!("content_filter")
    );
    let lengths = [
        json!({"type": "response.incomplete"}),
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}}),
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": null}}}),
    ]
    .map(|source| translated(&source).unwrap()["choices"][0]["finish_reason"].clone());
    assert_eq!(lengths, vec![json!("length"); 3]);
}

#[test]
fn stateless_translation_carries_usage_and_reasoning_items_on_completion() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "id": "resp_a",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "t"}]},
                {"type": "message"},
                7,
            ],
            "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3},
        },
    }))
    .unwrap();
    assert_eq!(chunk["choices"][0]["finish_reason"], json!("stop"));
    assert_eq!(
        chunk["choices"][0]["delta"]["reasoning_items"],
        json!([{"type": "reasoning", "id": "rs_1", "encrypted_content": null, "summary": [{"type": "summary_text", "text": "t"}]}])
    );
    assert_eq!(
        chunk["usage"],
        json!({
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3,
            "prompt_tokens_details": null,
            "completion_tokens_details": null,
        })
    );
}

#[test]
fn usage_details_follow_the_shared_projection() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 2,
                "output_tokens": 5,
                "total_tokens": 7,
                "input_tokens_details": {"cached_tokens": 1},
                "output_tokens_details": {"reasoning_tokens": 3},
            },
        },
    }))
    .unwrap();
    assert_eq!(
        chunk["usage"]["prompt_tokens_details"]["cached_tokens"],
        json!(1)
    );
    assert_eq!(
        chunk["usage"]["completion_tokens_details"]["reasoning_tokens"],
        json!(3)
    );
}

#[test]
fn terminal_usage_keeps_response_api_cache_keys_verbatim() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 2,
                "output_tokens": 5,
                "input_tokens_details": {"cached_tokens": 1},
                "cache_read_input_tokens": 7,
                "cache_creation_input_tokens": 3,
            },
        },
    }))
    .unwrap();
    let usage = &chunk["usage"];
    assert_eq!(usage["prompt_tokens"], json!(2));
    assert_eq!(usage["cache_read_input_tokens"], json!(7));
    assert_eq!(usage["cache_creation_input_tokens"], json!(3));
    let details = &usage["prompt_tokens_details"];
    assert_eq!(details["cached_tokens"], json!(7));
    assert_eq!(details["cache_write_tokens"], json!(3));
    assert_eq!(details["cache_creation_tokens"], json!(3));
    assert_eq!(details["cached_tokens_details"], json!(null));
}

#[test]
fn terminal_usage_folds_cache_counters_into_the_input_token_details_alias() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 2,
                "output_tokens": 5,
                "input_token_details": {"audio_tokens": 4},
                "cache_read_input_tokens": 7,
            },
        },
    }))
    .unwrap();
    let details = &chunk["usage"]["prompt_tokens_details"];
    assert_eq!(details["audio_tokens"], json!(4));
    assert_eq!(details["cached_tokens"], json!(7));
}

#[test]
fn stateless_translation_keeps_the_provider_metadata() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {"id": "resp_a", "custom_key": 5, "service_tier": "scale", "output": []},
    }))
    .unwrap();
    assert_eq!(
        chunk["provider_specific_fields"],
        json!({"custom_key": 5, "service_tier": "scale"})
    );
    assert_eq!(chunk["service_tier"], json!("scale"));
}

#[test]
fn stateless_translation_keeps_an_empty_terminal_tier() {
    let chunk = translated(&json!({
        "type": "response.completed",
        "response": {"incomplete_details": null, "service_tier": ""},
    }))
    .unwrap();
    assert_eq!(chunk["service_tier"], json!(""));
}
