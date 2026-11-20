#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use crate::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget, ScannedText, ScannedToolCall, ToolUseInputTarget,
};
use crate::guardrails::tool_call_identity::ToolCallShape;

use super::error::{ApplyError, WriteBackError};
use super::{
    apply_responses_to_input, apply_tool_calls_to_input, rewritten_tool_use_input,
    write_back_message_text, write_back_system_block, write_back_tool_use,
};

fn fields(value: &mut Value) -> &mut Map<String, Value> {
    value.as_object_mut().unwrap()
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

#[test]
fn system_block_rewrites_the_nth_text_block() {
    let mut system = json!([
        {"type": "text", "text": "first"},
        {"type": "image", "source": {"type": "url", "url": "https://example.test/i.png"}},
        {"type": "text", "text": "second"}
    ]);
    write_back_system_block(&mut system, 1, "redacted");
    assert_eq!(system[0]["text"], "first");
    assert_eq!(system[1]["source"]["url"], "https://example.test/i.png");
    assert_eq!(system[2]["text"], "redacted");
}

#[test]
fn system_block_skips_non_text_blocks_when_counting() {
    let mut system = json!([
        {"type": "image", "text": "not a text block"},
        {"type": "text", "text": "first"},
        {"type": "text", "text": "second"}
    ]);
    write_back_system_block(&mut system, 0, "redacted");
    assert_eq!(system[0]["text"], "not a text block");
    assert_eq!(system[1]["text"], "redacted");
    assert_eq!(system[2]["text"], "second");
}

#[test]
fn system_block_ignores_a_string_system_prompt() {
    let mut system = json!("keep the string");
    write_back_system_block(&mut system, 0, "redacted");
    assert_eq!(system, json!("keep the string"));
}

#[test]
fn system_block_ignores_an_index_past_the_text_blocks() {
    let mut system = json!([{"type": "text", "text": "only"}]);
    write_back_system_block(&mut system, 1, "redacted");
    assert_eq!(system, json!([{"type": "text", "text": "only"}]));
}

#[test]
fn message_text_replaces_a_string_content() {
    let mut message = json!({"role": "user", "content": "old"});
    write_back_message_text(
        fields(&mut message),
        &MessageTextTarget::MessageContent { msg_idx: 0 },
        "new",
    )
    .unwrap();
    assert_eq!(message["content"], "new");
}

#[test]
fn message_text_leaves_a_missing_or_null_content_alone() {
    let mut missing = json!({"role": "user"});
    write_back_message_text(
        fields(&mut missing),
        &MessageTextTarget::ContentBlockText {
            msg_idx: 0,
            content_idx: 0,
        },
        "new",
    )
    .unwrap();
    assert_eq!(missing, json!({"role": "user"}));

    let mut null = json!({"role": "user", "content": null});
    write_back_message_text(
        fields(&mut null),
        &MessageTextTarget::MessageContent { msg_idx: 0 },
        "new",
    )
    .unwrap();
    assert_eq!(null, json!({"role": "user", "content": null}));
}

#[test]
fn message_text_replaces_a_content_block_text() {
    let mut message = json!({"role": "assistant", "content": [
        text_block("old"),
        {"type": "tool_use", "id": "t1", "name": "lookup", "input": {"q": "kept"}}
    ]});
    write_back_message_text(
        fields(&mut message),
        &MessageTextTarget::ContentBlockText {
            msg_idx: 1,
            content_idx: 0,
        },
        "new",
    )
    .unwrap();
    assert_eq!(message["content"][0]["text"], "new");
    assert_eq!(message["content"][1]["input"], json!({"q": "kept"}));
}

#[test]
fn message_text_overwrites_a_tool_result_content_whatever_it_held() {
    let mut message = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": [text_block("old")]}
    ]});
    write_back_message_text(
        fields(&mut message),
        &MessageTextTarget::ToolResultString {
            msg_idx: 0,
            content_idx: 0,
        },
        "new",
    )
    .unwrap();
    assert_eq!(message["content"][0]["content"], "new");
}

#[test]
fn message_text_replaces_a_nested_tool_result_text() {
    let mut message = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": [text_block("first"), text_block("second")]}
    ]});
    write_back_message_text(
        fields(&mut message),
        &MessageTextTarget::ToolResultBlockText {
            msg_idx: 0,
            content_idx: 0,
            block_idx: 1,
        },
        "new",
    )
    .unwrap();
    assert_eq!(message["content"][0]["content"][0]["text"], "first");
    assert_eq!(message["content"][0]["content"][1]["text"], "new");
}

#[test]
fn message_text_leaves_a_container_that_does_not_fit_the_target() {
    let mut string_content = json!({"content": "flat"});
    write_back_message_text(
        fields(&mut string_content),
        &MessageTextTarget::ContentBlockText {
            msg_idx: 0,
            content_idx: 0,
        },
        "new",
    )
    .unwrap();
    assert_eq!(string_content, json!({"content": "flat"}));

    let mut array_content = json!({"content": [text_block("old")]});
    write_back_message_text(
        fields(&mut array_content),
        &MessageTextTarget::MessageContent { msg_idx: 0 },
        "new",
    )
    .unwrap();
    assert_eq!(array_content, json!({"content": [text_block("old")]}));
}

#[test]
fn message_text_errors_when_the_block_index_is_missing() {
    let mut message = json!({"content": [text_block("only")]});
    let target = MessageTextTarget::ContentBlockText {
        msg_idx: 3,
        content_idx: 1,
    };
    let error = write_back_message_text(fields(&mut message), &target, "new").unwrap_err();
    assert_eq!(error, WriteBackError::TargetMissing { target });
}

#[test]
fn message_text_errors_when_the_block_is_not_an_object() {
    let mut message = json!({"content": ["flat block"]});
    let target = MessageTextTarget::ToolResultString {
        msg_idx: 0,
        content_idx: 0,
    };
    let error = write_back_message_text(fields(&mut message), &target, "new").unwrap_err();
    assert_eq!(error, WriteBackError::TargetMissing { target });
}

#[test]
fn message_text_errors_when_the_tool_result_content_is_not_an_array() {
    let mut message = json!({"content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": "flat"}
    ]});
    let target = MessageTextTarget::ToolResultBlockText {
        msg_idx: 0,
        content_idx: 0,
        block_idx: 0,
    };
    let error = write_back_message_text(fields(&mut message), &target, "new").unwrap_err();
    assert_eq!(error, WriteBackError::TargetMissing { target });
}

#[test]
fn responses_to_input_writes_the_system_string_only_over_a_string() {
    let scanned = [ScannedText {
        text: "old".to_owned(),
        target: InputWriteBackTarget::SystemString,
    }];

    let mut string_system = json!({"system": "old", "messages": []});
    apply_responses_to_input(fields(&mut string_system), &["new".to_owned()], &scanned).unwrap();
    assert_eq!(string_system["system"], "new");

    let mut block_system = json!({"system": [text_block("old")]});
    apply_responses_to_input(fields(&mut block_system), &["new".to_owned()], &scanned).unwrap();
    assert_eq!(block_system["system"], json!([text_block("old")]));
}

#[test]
fn responses_to_input_pairs_to_the_shorter_list() {
    let mut data = json!({
        "system": [text_block("a"), text_block("b")],
        "messages": [{"content": "m"}]
    });
    let scanned = [
        ScannedText {
            text: "a".to_owned(),
            target: InputWriteBackTarget::SystemBlockText { block_idx: 0 },
        },
        ScannedText {
            text: "m".to_owned(),
            target: InputWriteBackTarget::Message(MessageTextTarget::MessageContent { msg_idx: 0 }),
        },
        ScannedText {
            text: "x".to_owned(),
            target: InputWriteBackTarget::SystemString,
        },
    ];
    apply_responses_to_input(
        fields(&mut data),
        &["A".to_owned(), "M".to_owned()],
        &scanned,
    )
    .unwrap();
    assert_eq!(data["system"][0]["text"], "A");
    assert_eq!(data["system"][1]["text"], "b");
    assert_eq!(data["messages"][0]["content"], "M");
}

#[test]
fn responses_to_input_writes_at_the_target_message_index() {
    let first = [ScannedText {
        text: "first".to_owned(),
        target: InputWriteBackTarget::Message(MessageTextTarget::MessageContent { msg_idx: 0 }),
    }];
    let mut zero = json!({"messages": [{"content": "first"}, {"content": "second"}]});
    apply_responses_to_input(fields(&mut zero), &["NEW".to_owned()], &first).unwrap();
    assert_eq!(zero["messages"][0]["content"], "NEW");
    assert_eq!(zero["messages"][1]["content"], "second");

    let second = [ScannedText {
        text: "second".to_owned(),
        target: InputWriteBackTarget::Message(MessageTextTarget::MessageContent { msg_idx: 1 }),
    }];
    let mut one = json!({"messages": [{"content": "first"}, {"content": "second"}]});
    apply_responses_to_input(fields(&mut one), &["NEW".to_owned()], &second).unwrap();
    assert_eq!(one["messages"][0]["content"], "first");
    assert_eq!(one["messages"][1]["content"], "NEW");
}

#[test]
fn responses_to_input_writes_system_blocks_and_messages() {
    let mut data = json!({
        "system": [{"type": "image", "source": {"type": "url", "url": "https://example.test/i.png"}}, text_block("old")],
        "messages": [{"role": "user", "content": [text_block("old")]}]
    });
    let scanned = [
        ScannedText {
            text: "old".to_owned(),
            target: InputWriteBackTarget::SystemBlockText { block_idx: 0 },
        },
        ScannedText {
            text: "old".to_owned(),
            target: InputWriteBackTarget::Message(MessageTextTarget::ContentBlockText {
                msg_idx: 0,
                content_idx: 0,
            }),
        },
    ];
    apply_responses_to_input(
        fields(&mut data),
        &["S".to_owned(), "M".to_owned()],
        &scanned,
    )
    .unwrap();
    assert_eq!(data["system"][1]["text"], "S");
    assert_eq!(data["messages"][0]["content"][0]["text"], "M");
}

#[rstest]
#[case::short(json!({"messages": [{"content": "only"}]}))]
#[case::no_messages(json!({"system": "s"}))]
#[case::scalar_messages(json!({"messages": "not a list"}))]
#[test]
fn responses_to_input_errors_when_the_message_is_missing(#[case] mut data: Value) {
    let target = MessageTextTarget::MessageContent { msg_idx: 2 };
    let scanned = [ScannedText {
        text: "x".to_owned(),
        target: InputWriteBackTarget::Message(target.clone()),
    }];
    let error =
        apply_responses_to_input(fields(&mut data), &["X".to_owned()], &scanned).unwrap_err();
    assert_eq!(
        error,
        WriteBackError::TargetMissing {
            target: target.clone()
        }
    );
}

#[test]
fn responses_to_input_errors_when_the_message_row_is_not_an_object() {
    let target = MessageTextTarget::MessageContent { msg_idx: 0 };
    let scanned = [ScannedText {
        text: "x".to_owned(),
        target: InputWriteBackTarget::Message(target.clone()),
    }];
    let mut data = json!({"messages": ["not a message"]});
    let error =
        apply_responses_to_input(fields(&mut data), &["X".to_owned()], &scanned).unwrap_err();
    assert_eq!(error, WriteBackError::TargetMissing { target });
}

#[test]
fn tool_use_accepts_only_a_json_object() {
    assert_eq!(rewritten_tool_use_input("{}"), Some(Map::new()));
    assert_eq!(
        rewritten_tool_use_input(r#"{"q": "new"}"#)
            .unwrap()
            .get("q"),
        Some(&json!("new"))
    );
    assert_eq!(rewritten_tool_use_input("[]"), None);
    assert_eq!(rewritten_tool_use_input(r#""text""#), None);
    assert_eq!(rewritten_tool_use_input("null"), None);
    assert_eq!(rewritten_tool_use_input(""), None);
    assert_eq!(rewritten_tool_use_input("not json"), None);
}

#[test]
fn tool_use_writes_the_input_and_keeps_an_equal_name() {
    let mut message = json!({"role": "assistant", "content": [
        {"type": "tool_use", "id": "t1", "name": "lookup", "input": {"q": "old"}}
    ]});
    let shape = ToolCallShape {
        name: Some("lookup".to_owned()),
        arguments: "{}".to_owned(),
    };
    let mut rewritten_input = Map::new();
    rewritten_input.insert("q".to_owned(), json!("new"));
    write_back_tool_use(
        fields(&mut message),
        &ToolUseInputTarget {
            msg_idx: 0,
            content_idx: 0,
        },
        &shape,
        rewritten_input,
    );
    assert_eq!(message["content"][0]["input"], json!({"q": "new"}));
    assert_eq!(message["content"][0]["name"], "lookup");
}

#[test]
fn tool_use_renames_only_when_the_shape_name_differs() {
    let mut message = json!({"content": [
        {"type": "tool_use", "id": "t1", "name": "old", "input": {"q": "kept"}}
    ]});
    let shape = ToolCallShape {
        name: Some("new".to_owned()),
        arguments: "{}".to_owned(),
    };
    write_back_tool_use(
        fields(&mut message),
        &ToolUseInputTarget {
            msg_idx: 0,
            content_idx: 0,
        },
        &shape,
        Map::new(),
    );
    assert_eq!(message["content"][0]["name"], "new");
    assert_eq!(message["content"][0]["input"], json!({}));
}

#[test]
fn tool_use_keeps_the_name_when_the_shape_has_none() {
    let mut message = json!({"content": [
        {"type": "tool_use", "id": "t1", "name": "old", "input": {}}
    ]});
    let shape = ToolCallShape {
        name: None,
        arguments: "{}".to_owned(),
    };
    write_back_tool_use(
        fields(&mut message),
        &ToolUseInputTarget {
            msg_idx: 0,
            content_idx: 0,
        },
        &shape,
        Map::new(),
    );
    assert_eq!(message["content"][0]["name"], "old");
}

#[test]
fn tool_use_ignores_a_message_it_cannot_write_into() {
    let shape = ToolCallShape {
        name: Some("new".to_owned()),
        arguments: "{}".to_owned(),
    };
    let target = ToolUseInputTarget {
        msg_idx: 0,
        content_idx: 0,
    };

    let mut string_content = json!({"content": "flat"});
    write_back_tool_use(fields(&mut string_content), &target, &shape, Map::new());
    assert_eq!(string_content, json!({"content": "flat"}));

    let mut empty_content = json!({"content": []});
    write_back_tool_use(fields(&mut empty_content), &target, &shape, Map::new());
    assert_eq!(empty_content, json!({"content": []}));

    let mut scalar_block = json!({"content": ["flat block"]});
    write_back_tool_use(fields(&mut scalar_block), &target, &shape, Map::new());
    assert_eq!(scalar_block, json!({"content": ["flat block"]}));
}

fn scanned_pair() -> ([ScannedToolCall; 2], [ToolCallShape; 2], Vec<Value>) {
    let scanned = [
        ScannedToolCall {
            tool_call: json!({"id": "t1", "function": {"name": "one", "arguments": "{}"}}),
            target: ToolUseInputTarget {
                msg_idx: 0,
                content_idx: 0,
            },
        },
        ScannedToolCall {
            tool_call: json!({"id": "t2", "function": {"name": "two", "arguments": "{}"}}),
            target: ToolUseInputTarget {
                msg_idx: 0,
                content_idx: 1,
            },
        },
    ];
    let pre = [
        ToolCallShape {
            name: Some("one".to_owned()),
            arguments: "{}".to_owned(),
        },
        ToolCallShape {
            name: Some("two".to_owned()),
            arguments: "{}".to_owned(),
        },
    ];
    let messages = vec![json!({"role": "assistant", "content": [
        {"type": "tool_use", "id": "t1", "name": "one", "input": {}},
        {"type": "tool_use", "id": "t2", "name": "two", "input": {"kept": true}}
    ]})];
    (scanned, pre, messages)
}

#[test]
fn tool_calls_rewrites_only_changed_shapes() {
    let (scanned, pre, mut messages) = scanned_pair();
    let returned = vec![
        json!({"function": {"name": "renamed", "arguments": "{\"q\":\"new\"}"}}),
        json!({"function": {"name": "two", "arguments": "{}"}}),
    ];
    apply_tool_calls_to_input(
        &mut messages,
        &scanned,
        &pre,
        Some(&returned),
        Some("guard"),
    )
    .unwrap();
    assert_eq!(messages[0]["content"][0]["input"], json!({"q": "new"}));
    assert_eq!(messages[0]["content"][0]["name"], "renamed");
    assert_eq!(messages[0]["content"][1]["input"], json!({"kept": true}));
    assert_eq!(messages[0]["content"][1]["name"], "two");
}

#[test]
fn tool_calls_ignores_a_returned_list_of_the_wrong_length() {
    let (scanned, pre, mut messages) = scanned_pair();
    let returned = vec![json!({"function": {"name": "renamed", "arguments": "{\"q\":\"new\"}"}})];
    apply_tool_calls_to_input(
        &mut messages,
        &scanned,
        &pre,
        Some(&returned),
        Some("guard"),
    )
    .unwrap();
    assert_eq!(messages[0]["content"][0]["input"], json!({}));
    assert_eq!(messages[0]["content"][0]["name"], "one");

    let (scanned, pre, mut absent_answer) = scanned_pair();
    apply_tool_calls_to_input(&mut absent_answer, &scanned, &pre, None, Some("guard")).unwrap();
    assert_eq!(absent_answer[0]["content"][0]["input"], json!({}));
    assert_eq!(
        absent_answer[0]["content"][1]["input"],
        json!({"kept": true})
    );
}

#[test]
fn tool_calls_stops_at_the_shortest_list() {
    let (scanned, pre, mut messages) = scanned_pair();
    let returned = vec![json!({"function": {"name": "renamed", "arguments": "{\"q\":\"new\"}"}})];
    apply_tool_calls_to_input(&mut messages, &scanned, &pre[..1], Some(&returned), None).unwrap();
    assert_eq!(messages[0]["content"][0]["input"], json!({"q": "new"}));
    assert_eq!(messages[0]["content"][0]["name"], "renamed");
    assert_eq!(messages[0]["content"][1]["input"], json!({"kept": true}));
}

#[test]
fn tool_calls_rejects_an_array_argument_whole() {
    let (scanned, pre, mut messages) = scanned_pair();
    let returned = vec![
        json!({"function": {"name": "one", "arguments": "{\"q\":\"new\"}"}}),
        json!({"function": {"name": "two", "arguments": "[]"}}),
    ];
    let error = apply_tool_calls_to_input(&mut messages, &scanned, &pre, Some(&returned), None)
        .unwrap_err();
    assert_eq!(
        error,
        ApplyError::Unappliable(UnappliableRequestRewrite::new(None))
    );
    assert_eq!(messages[0]["content"][0]["input"], json!({}));
    assert_eq!(messages[0]["content"][0]["name"], "one");
    assert_eq!(messages[0]["content"][1]["input"], json!({"kept": true}));
}

#[test]
fn tool_calls_leaves_an_unchanged_invalid_argument_list_alone() {
    let scanned = [ScannedToolCall {
        tool_call: json!({"id": "t1", "function": {"name": "one", "arguments": "[]"}}),
        target: ToolUseInputTarget {
            msg_idx: 0,
            content_idx: 0,
        },
    }];
    let pre = [ToolCallShape {
        name: Some("one".to_owned()),
        arguments: "[]".to_owned(),
    }];
    let returned = vec![json!({"function": {"name": "one", "arguments": "[]"}})];
    let mut messages = vec![json!({"content": [
        {"type": "tool_use", "id": "t1", "name": "one", "input": {"kept": true}}
    ]})];
    apply_tool_calls_to_input(&mut messages, &scanned, &pre, Some(&returned), None).unwrap();
    assert_eq!(messages[0]["content"][0]["input"], json!({"kept": true}));
}

#[test]
fn tool_calls_errors_when_the_message_is_missing() {
    let (scanned, pre, _messages) = scanned_pair();
    let returned = vec![
        json!({"function": {"name": "one", "arguments": "{\"q\":\"new\"}"}}),
        json!({"function": {"name": "two", "arguments": "{}"}}),
    ];
    let mut empty: Vec<Value> = Vec::new();
    let error =
        apply_tool_calls_to_input(&mut empty, &scanned, &pre, Some(&returned), None).unwrap_err();
    assert_eq!(error, ApplyError::MessageMissing { msg_idx: 0 });

    let mut scalar_message = vec![json!("not a message")];
    let error =
        apply_tool_calls_to_input(&mut scalar_message, &scanned, &pre, Some(&returned), None)
            .unwrap_err();
    assert_eq!(error, ApplyError::MessageMissing { msg_idx: 0 });
}
