#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::support::{choice, optional_params, parse, text, tool_call, tools, value};
use crate::chat_choices::ChatChoicesError;
use crate::chat_choices::wire::{CustomPayload, TransformedChoice};
use crate::openai_base_config::Object;

const NIL_ID: &str = "00000000-0000-0000-0000-000000000000";
const CONTENT: &str = r#"{"type":"function","name":"lookup","arguments":"{\"city\":\"Paris\"}"}"#;

#[rstest]
#[case::tool_call_json_content(0)]
#[case::tool_calls_finish_reason(1)]
#[case::empty_choice(2)]
#[case::reasoning_content_delta(3)]
#[case::embedded_stream_error(4)]
#[case::response_headers(5)]
#[case::model_list(6)]
#[case::query_url_join(7)]
#[case::stream_envelope_fields(8)]
#[case::chat_auth_content_type(9)]
#[case::model_list_decode_failure(10)]
#[case::credential_accessor_defaults(11)]
#[rustfmt::skip]
fn chat_choices_cases(#[case] case: u8) {
    match case {
        0 => {
            tool_call_json_content();
            tool_call_json_content_precedence();
        }
        1 => tool_calls_finish_reason(),
        2 => empty_choice(),
        3 => super::stream::reasoning_content_delta(),
        4 => super::stream::embedded_stream_error(),
        5 => super::reply::response_headers(),
        6 => super::endpoint::model_list(),
        7 => super::endpoint::query_url_join(),
        8 => super::stream::stream_envelope_fields(),
        9 => super::endpoint::chat_auth_content_type(),
        10 => super::endpoint::model_list_decode_failure(),
        11 => super::endpoint::credential_accessor_defaults(),
        _ => panic!("unknown case {case}"),
    }
}

fn content_message(content: &str) -> Value {
    value(&[("content", text(content))])
}

fn content_choice(content: &str) -> Value {
    choice(0, Some("stop"), content_message(content))
}

fn only(choices: &[Value], params: Option<&Object>, json_mode: bool) -> TransformedChoice {
    let mut read = parse(choices, params, json_mode).unwrap();
    assert_eq!(read.len(), 1);
    read.remove(0)
}

fn tool_call_json_content() {
    let params = tools(&["lookup"]);
    let converted = only(&[content_choice(CONTENT)], Some(&params), false);
    assert_eq!(converted.finish_reason, "tool_calls");
    assert_eq!(converted.message.content, None);
    let call = &converted.message.tool_calls.as_ref().unwrap()[0];
    assert_eq!(call.id, NIL_ID);
    assert_eq!(call.kind, "function");
    assert_eq!(
        call.function.as_ref().unwrap().name.as_deref(),
        Some("lookup")
    );
    assert_eq!(
        call.function.as_ref().unwrap().arguments,
        r#"{"city":"Paris"}"#
    );
    assert!(converted.provider_specific_fields.is_none());

    // tools is None: the content stays text
    let plain = only(&[content_choice(CONTENT)], None, false);
    assert_eq!(plain.message.content.as_deref(), Some(CONTENT));
    assert!(plain.message.tool_calls.is_none());
    assert_eq!(plain.finish_reason, "stop");

    malformed_content_stays_text(&params);
}

fn malformed_content_stays_text(params: &Object) {
    for remaining in [
        "not json",
        r#"{"type":"function","name":"other","arguments":""}"#,
    ] {
        let kept = only(&[content_choice(remaining)], Some(params), false);
        assert_eq!(kept.message.content.as_deref(), Some(remaining));
        assert!(kept.message.tool_calls.is_none());
    }
}

fn tool_call_json_content_precedence() {
    let params = tools(&["lookup"]);
    let empty_calls = choice(
        0,
        Some("stop"),
        value(&[
            ("content", text(CONTENT)),
            ("tool_calls", Value::Array(Vec::new())),
        ]),
    );
    let empty_read = only(&[empty_calls], Some(&params), false);
    assert_eq!(empty_read.message.content.as_deref(), Some(CONTENT));
    assert!(empty_read.message.tool_calls.is_none());
    let native = choice(
        0,
        Some("stop"),
        value(&[
            ("content", text(CONTENT)),
            (
                "tool_calls",
                Value::Array(vec![tool_call(Some("call_a"), "lookup", "{}")]),
            ),
        ]),
    );
    let native_read = only(&[native], Some(&params), false);
    assert_eq!(native_read.message.content.as_deref(), Some(CONTENT));
    assert_eq!(
        native_read.message.tool_calls.as_ref().unwrap()[0].id,
        "call_a"
    );

    let empty = only(&[content_choice("")], Some(&params), false);
    assert_eq!(empty.message.content.as_deref(), Some(""));
    assert!(empty.message.tool_calls.is_none());

    let null_tools = optional_params(&[("tools", Value::Null)]);
    let null_read = only(&[content_choice(CONTENT)], Some(&null_tools), false);
    assert_eq!(null_read.message.content.as_deref(), Some(CONTENT));
    assert!(null_read.message.tool_calls.is_none());
    let without_tools = optional_params(&[("temperature", Value::from(1_i64))]);
    let kept = only(&[content_choice(CONTENT)], Some(&without_tools), false);
    assert_eq!(kept.message.content.as_deref(), Some(CONTENT));
    content_arguments(&params);
}

fn content_arguments(params: &Object) {
    let head = r#"{"type":"function","name":"lookup""#;
    for (tail, expected) in [
        ("}", Some("")),
        (r#","arguments":null}"#, Some("")),
        (r#","arguments":{"a":1}}"#, Some(r#"{"a":1}"#)),
        (r#","arguments":[1]}"#, None),
        (r#","arguments":3}"#, None),
        (r#","arguments":true}"#, None),
    ] {
        let content = format!("{head}{tail}");
        let read = only(&[content_choice(&content)], Some(params), false);
        match expected {
            Some(arguments) => {
                assert_eq!(read.message.content, None, "{content}");
                let call = &read.message.tool_calls.as_ref().unwrap()[0];
                assert_eq!(call.function.as_ref().unwrap().arguments, arguments);
            }
            None => assert_eq!(read.message.content.as_deref(), Some(content.as_str())),
        }
    }
    for ignored in [
        r#"{"type":"tool","name":"lookup"}"#,
        r#"{"type":"function","name":"other"}"#,
    ] {
        let read = only(&[content_choice(ignored)], Some(params), false);
        assert_eq!(read.message.content.as_deref(), Some(ignored));
        assert!(read.message.tool_calls.is_none());
    }
}

fn tool_calls_finish_reason() {
    native_tool_calls();
    parallel_tool_calls();
    json_mode_tool_calls();
    mapped_finish_reason();
    custom_tool_calls();
    broken_custom_tool_calls();
}

fn custom_tool_calls() {
    let custom = value(&[
        ("id", text("call_c")),
        (
            "custom",
            value(&[("name", text("patch")), ("input", text("diff"))]),
        ),
    ]);
    let read = only(
        &[choice(
            0,
            Some("stop"),
            value(&[("tool_calls", Value::Array(vec![custom]))]),
        )],
        None,
        false,
    );
    assert_eq!(read.finish_reason, "tool_calls");
    let call = &read.message.tool_calls.as_ref().unwrap()[0];
    assert_eq!((call.id.as_str(), call.kind.as_str()), ("call_c", "custom"));
    assert!(call.function.is_none());
    assert_eq!(
        call.custom,
        Some(CustomPayload {
            name: String::from("patch"),
            input: String::from("diff"),
        })
    );
}

fn broken_custom_tool_calls() {
    for (broken, expected) in [
        (
            value(&[("id", text("c")), ("type", text("custom"))]),
            "MissingCustom",
        ),
        (
            value(&[
                ("type", text("custom")),
                (
                    "custom",
                    value(&[("name", text("n")), ("input", text("i"))]),
                ),
            ]),
            "MissingCustomId",
        ),
        (
            value(&[("id", text("c")), ("type", text("function"))]),
            "MissingFunction",
        ),
    ] {
        let choices = [choice(
            0,
            Some("stop"),
            value(&[("tool_calls", Value::Array(vec![broken]))]),
        )];
        let Err(ChatChoicesError::Choices(error)) = parse(&choices, None, false) else {
            panic!("{expected} expected");
        };
        assert_eq!(format!("{error:?}"), expected);
    }
}

fn native_tool_calls() {
    let native = choice(
        0,
        Some("stop"),
        value(&[(
            "tool_calls",
            Value::Array(vec![tool_call(
                Some("call_a"),
                "lookup",
                r#"{"city":"Paris"}"#,
            )]),
        )]),
    );
    let read = only(&[native], None, false);
    assert_eq!(read.finish_reason, "tool_calls");
    let function = read.message.tool_calls.as_ref().unwrap()[0]
        .function
        .as_ref()
        .unwrap();
    assert_eq!(read.message.tool_calls.as_ref().unwrap()[0].id, "call_a");
    assert_eq!(function.arguments, r#"{"city":"Paris"}"#);
}

fn parallel_tool_uses() -> Value {
    let uses = [
        value(&[
            ("recipient_name", text("functions.alpha")),
            ("parameters", value(&[])),
        ]),
        value(&[("recipient_name", text("beta")), ("parameters", value(&[]))]),
    ];
    value(&[("tool_uses", Value::Array(uses.to_vec()))])
}

fn parallel_tool_calls() {
    let repaired = choice(
        0,
        Some("tool_calls"),
        value(&[(
            "tool_calls",
            Value::Array(vec![
                tool_call(
                    Some("call_1"),
                    "multi_tool_use.parallel",
                    &parallel_tool_uses().to_string(),
                ),
                tool_call(Some("call_2"), "gamma", "{}"),
            ]),
        )]),
    );
    let read = only(&[repaired], None, false);
    assert_eq!(read.finish_reason, "tool_calls");
    let calls = read.message.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].id, "call_1_0");
    assert_eq!(
        calls[0].function.as_ref().unwrap().name.as_deref(),
        Some("alpha")
    );
    assert_eq!(calls[0].function.as_ref().unwrap().arguments, "{}");
    assert_eq!(calls[1].id, "call_1_1");
    assert_eq!(
        calls[1].function.as_ref().unwrap().name.as_deref(),
        Some("beta")
    );
    assert_eq!(calls[2].id, "call_2");
    unparsable_arguments_keep_the_calls();
    tool_call_keys_pass_through();
}

fn tool_call_keys_pass_through() {
    let mut call = tool_call(Some("call_a"), "lookup", "{}");
    call["index"] = Value::from(3_i64);
    let carried = choice(
        0,
        Some("stop"),
        value(&[("tool_calls", Value::Array(vec![call]))]),
    );
    let read = only(&[carried], None, false);
    let kept = &read.message.tool_calls.as_ref().unwrap()[0];
    assert_eq!(kept.extra.get("index"), Some(&Value::from(3_i64)));
    let dumped = serde_json::to_value(kept).unwrap();
    assert_eq!(dumped["index"], Value::from(3_i64));
}

fn unparsable_arguments_keep_the_calls() {
    let broken = choice(
        0,
        Some("stop"),
        value(&[(
            "tool_calls",
            Value::Array(vec![tool_call(Some("call_a"), "lookup", "not json")]),
        )]),
    );
    let read = only(&[broken], None, false);
    assert_eq!(
        read.message.tool_calls.as_ref().unwrap()[0]
            .function
            .as_ref()
            .unwrap()
            .arguments,
        "not json"
    );
}

fn json_mode_tool_calls() {
    let single = choice(
        0,
        Some("length"),
        value(&[(
            "tool_calls",
            Value::Array(vec![tool_call(
                Some("call_j"),
                "json_tool_call",
                r#"{"a":1}"#,
            )]),
        )]),
    );
    let read = only(&[single], None, true);
    assert_eq!(read.message.content.as_deref(), Some(r#"{"a":1}"#));
    assert!(read.message.tool_calls.is_none());
    assert_eq!(read.finish_reason, "length");
    assert!(read.provider_specific_fields.is_none());

    let empty = choice(
        0,
        Some("stop"),
        value(&[(
            "tool_calls",
            Value::Array(vec![tool_call(Some("call_j"), "json_tool_call", "")]),
        )]),
    );
    let empty_read = only(&[empty], None, true);
    assert_eq!(empty_read.finish_reason, "tool_calls");
    assert!(empty_read.message.content.is_none());
    assert_eq!(
        empty_read.message.tool_calls.as_ref().unwrap()[0].id,
        "call_j"
    );
}

fn mapped_finish_reason() {
    let unmapped = choice(0, Some("stop_sequence"), value(&[("content", text("x"))]));
    let read = only(&[unmapped], None, false);
    assert_eq!(read.finish_reason, "stop");
    assert_eq!(
        read.provider_specific_fields
            .as_ref()
            .unwrap()
            .get("native_finish_reason"),
        Some(&text("stop_sequence"))
    );
}

fn empty_choice() {
    let empty = only(&[choice(0, Some("stop"), value(&[]))], None, false);
    assert_eq!(empty.index, 0);
    assert_eq!(empty.finish_reason, "stop");
    assert_eq!(empty.message.role, "assistant");
    assert_eq!(empty.message.content, None);
    assert!(empty.message.tool_calls.is_none());
    assert!(empty.message.reasoning_content.is_none());
    assert!(empty.provider_specific_fields.is_none());

    let null_finish = only(&[choice(3, None, value(&[]))], None, false);
    assert_eq!(null_finish.index, 3);
    assert_eq!(null_finish.finish_reason, "stop");
    message_reasoning();
}

fn message_reasoning() {
    let tagged = "<think>plan</think>answer";
    for (message, reasoning, content) in [
        (content_message(tagged), Some("plan"), Some("answer")),
        (
            value(&[
                ("content", text(tagged)),
                ("reasoning_content", text("given")),
            ]),
            Some("given"),
            Some(tagged),
        ),
        (
            value(&[("content", text(tagged)), ("reasoning", text("alt"))]),
            Some("alt"),
            Some(tagged),
        ),
        (
            value(&[
                ("content", text(tagged)),
                ("reasoning_content", Value::Null),
            ]),
            None,
            Some(tagged),
        ),
        (
            value(&[
                ("content", text(tagged)),
                ("reasoning_content", Value::Null),
                ("reasoning", text("alt")),
            ]),
            None,
            Some(tagged),
        ),
    ] {
        let read = only(&[choice(0, Some("stop"), message)], None, false);
        assert_eq!(read.message.reasoning_content.as_deref(), reasoning);
        assert_eq!(read.message.content.as_deref(), content);
    }
}
