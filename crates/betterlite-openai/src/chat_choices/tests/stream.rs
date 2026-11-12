#![cfg(test)]

use serde_json::Value;

use super::support::{fixed_clock, nil_id, text, value};
use crate::chat_choices::stream::{
    CHUNK_OBJECT, StreamError, chunk_parser, extract_error_from_chunk,
};
use crate::chat_choices::wire::{StreamDelta, StreamReply};

fn parse(chunk: &Value) -> Result<StreamReply, StreamError> {
    chunk_parser(chunk, nil_id, fixed_clock)
}

fn envelope(choices: Value) -> Value {
    value(&[
        ("id", text("c1")),
        ("created", Value::from(7_i64)),
        ("model", text("m")),
        ("choices", choices),
    ])
}

fn delta_choices(delta: Value) -> Value {
    Value::Array(vec![value(&[
        ("index", Value::from(0_i64)),
        ("delta", delta),
    ])])
}

fn reasoning_delta(reasoning: Value) -> Value {
    value(&[("reasoning", reasoning)])
}

pub(super) fn reasoning_content_delta() {
    let moved = parse(&envelope(delta_choices(reasoning_delta(text("r1"))))).unwrap();
    assert_eq!(
        moved.choices[0].delta.reasoning_content.as_deref(),
        Some("r1")
    );
    assert!(moved.choices[0].delta.passthrough.is_empty());
    overwritten_reasoning();
    null_reasoning();
    absent_delta_or_choices();
}

fn overwritten_reasoning() {
    let delta = value(&[
        ("reasoning_content", text("old")),
        ("reasoning", text("new")),
    ]);
    let overwritten = parse(&envelope(delta_choices(delta))).unwrap();
    assert_eq!(
        overwritten.choices[0].delta.reasoning_content.as_deref(),
        Some("new")
    );
}

fn null_reasoning() {
    let null_reasoning = parse(&envelope(delta_choices(value(&[(
        "reasoning",
        Value::Null,
    )]))))
    .unwrap();
    assert!(null_reasoning.choices[0].delta.reasoning_content.is_none());
}

fn absent_delta_or_choices() {
    let absent = parse(&envelope(Value::Array(vec![value(&[])]))).unwrap();
    assert_eq!(absent.choices.len(), 1);
    assert_eq!(absent.choices[0].delta, StreamDelta::default());
    let no_choices = value(&[
        ("id", text("c1")),
        ("created", Value::from(7_i64)),
        ("model", text("m")),
    ]);
    assert!(parse(&no_choices).unwrap().choices.is_empty());
    assert!(
        parse(&envelope(Value::Array(Vec::new())))
            .unwrap()
            .choices
            .is_empty()
    );
}

fn error_chunk(error: Value) -> Value {
    value(&[("error", error)])
}

fn error_status(chunk: &Value) -> Option<u16> {
    match parse(chunk) {
        Err(StreamError::Provider { status, .. }) => Some(status),
        _ => None,
    }
}

fn error_object(code: Value) -> Value {
    value(&[("message", text("x")), ("code", code)])
}

pub(super) fn embedded_stream_error() {
    let boom = value(&[("message", text("boom")), ("code", Value::from(400_i64))]);
    assert_eq!(
        extract_error_from_chunk(&boom),
        Some((String::from("boom"), 400))
    );
    assert_eq!(
        extract_error_from_chunk(&error_object(Value::from(599_i64))),
        Some((String::from("x"), 599))
    );
    status_bounds();
    falsy_errors_are_ignored();
    assert_eq!(error_status(&error_chunk(boom)), Some(400));
    assert_eq!(error_status(&error_chunk(text("plain"))), Some(500));
    assert_eq!(
        extract_error_from_chunk(&text("plain")),
        Some((String::from("plain"), 500))
    );
    assert_eq!(
        extract_error_from_chunk(&Value::Bool(true)).map(|(_, status)| status),
        Some(500)
    );
}

fn status_bounds() {
    for (code, expected) in [(600_i64, 500_u16), (399, 500), (500, 500)] {
        let error = error_chunk(error_object(Value::from(code)));
        assert_eq!(error_status(&error), Some(expected));
    }
    for code in [text("400"), Value::from(400.0_f64)] {
        assert_eq!(error_status(&error_chunk(error_object(code))), Some(500));
    }
}

fn falsy_errors_are_ignored() {
    for ignored in [
        Value::Null,
        Value::Bool(false),
        Value::from(0_i64),
        text(""),
        Value::Array(Vec::new()),
        value(&[]),
    ] {
        assert_eq!(extract_error_from_chunk(&ignored), None);
        let parsed = parse(&value(&[
            ("id", text("c")),
            ("created", Value::from(1_i64)),
            ("model", text("m")),
            ("error", ignored),
        ]))
        .unwrap();
        assert_eq!(parsed.id, "c");
    }
}

fn full_envelope() -> Value {
    let usage = value(&[
        ("prompt_tokens", Value::from(1_i64)),
        ("completion_tokens", Value::from(2_i64)),
        ("total_tokens", Value::from(3_i64)),
    ]);
    value(&[
        ("id", text("c1")),
        ("created", Value::from(7_i64)),
        ("model", text("m")),
        ("object", text("something.else")),
        ("choices", Value::Array(Vec::new())),
        ("usage", usage),
        ("service_tier", text("scale")),
        ("system_fingerprint", text("fp")),
    ])
}

pub(super) fn stream_envelope_fields() {
    let chunk = parse(&full_envelope()).unwrap();
    assert_eq!(chunk.id, "c1");
    assert_eq!(chunk.object, CHUNK_OBJECT);
    assert_eq!(chunk.created, 7);
    assert_eq!(chunk.model.as_deref(), Some("m"));
    assert!(chunk.choices.is_empty());
    let usage = chunk.usage.as_ref().unwrap();
    assert_eq!(usage["prompt_tokens"], Value::from(1_i64));
    assert_eq!(usage["completion_tokens"], Value::from(2_i64));
    assert_eq!(usage["total_tokens"], Value::from(3_i64));
    assert!(usage["prompt_tokens_details"].is_null());
    assert_eq!(chunk.service_tier.as_deref(), Some("scale"));
    generated_envelope();
    optional_envelope_fields();
}

fn generated_envelope() {
    let generated = parse(&value(&[
        ("choices", Value::Array(Vec::new())),
        ("model", Value::Null),
    ]))
    .unwrap();
    assert_eq!(generated.id, format!("chatcmpl-{}", nil_id()));
    assert_eq!(generated.created, 7);
    assert!(generated.model.is_none());
}

fn optional_envelope_fields() {
    let bare = parse(&value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("usage", Value::Null),
        ("service_tier", text("")),
    ]))
    .unwrap();
    assert!(bare.usage.is_none());
    assert!(bare.service_tier.is_none());
    let numeric_tier = parse(&value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("service_tier", Value::from(3_i64)),
    ]))
    .unwrap();
    assert!(numeric_tier.service_tier.is_none());
    let zeroed = parse(&value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("usage", value(&[])),
    ]))
    .unwrap();
    let usage = zeroed.usage.as_ref().unwrap();
    assert_eq!(usage["prompt_tokens"], Value::from(0_i64));
    assert_eq!(usage["completion_tokens"], Value::from(0_i64));
    assert_eq!(usage["total_tokens"], Value::from(0_i64));
    choice_finish_mapping();
}

fn choice_finish_mapping() {
    assert!(matches!(
        parse(&value(&[
            ("id", text("c")),
            ("created", Value::from(1_i64)),
            ("model", text("m")),
            ("choices", Value::Null),
        ])),
        Err(StreamError::Chunk(_))
    ));
    let mapped = parse(&envelope(Value::Array(vec![value(&[
        ("index", Value::from(2_i64)),
        ("finish_reason", text("stop_sequence")),
        ("delta", value(&[("content", text("x"))])),
        ("logprobs", Value::Null),
    ])])))
    .unwrap();
    assert_eq!(mapped.choices[0].index, 2);
    assert_eq!(mapped.choices[0].finish_reason.as_deref(), Some("stop"));
    let empty_reason = parse(&envelope(Value::Array(vec![value(&[(
        "finish_reason",
        text(""),
    )])])))
    .unwrap();
    assert!(empty_reason.choices[0].finish_reason.is_none());
    assert!(matches!(
        parse(&envelope(Value::Array(vec![Value::from(5_i64)]))),
        Err(StreamError::Chunk(_))
    ));
    usage_and_tool_calls();
}

fn usage_chunk(usage: Value) -> Value {
    value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("usage", usage),
    ])
}

fn usage_and_tool_calls() {
    let with_cost = parse(&usage_chunk(value(&[
        ("prompt_tokens", Value::Null),
        ("cost", Value::from(0.5_f64)),
        ("server_tool_use", Value::Null),
    ])))
    .unwrap();
    let usage = with_cost.usage.as_ref().unwrap();
    assert_eq!(usage["prompt_tokens"], Value::from(0_i64));
    assert_eq!(usage["cost"], Value::from(0.5_f64));
    assert!(usage.get("server_tool_use").is_none());
    let null_cost = parse(&usage_chunk(value(&[("cost", Value::Null)]))).unwrap();
    assert!(null_cost.usage.as_ref().unwrap().get("cost").is_none());
    folded_usage();
    numbered_tool_calls();
    dict_arguments();
    invalid_tool_calls();
}

fn folded_usage() {
    let folded = parse(&usage_chunk(value(&[
        ("prompt_tokens", Value::from(5_i64)),
        ("completion_tokens", Value::from(8_i64)),
        ("total_tokens", Value::from(13_i64)),
        ("reasoning_tokens", Value::from(3_i64)),
        ("prompt_cache_hit_tokens", Value::from(4_i64)),
        ("cache_creation_input_tokens", Value::from(2_i64)),
    ])))
    .unwrap();
    let usage = folded.usage.as_ref().unwrap();
    assert!(usage.get("reasoning_tokens").is_none());
    assert_eq!(
        usage["completion_tokens_details"],
        value(&[
            ("reasoning_tokens", Value::from(3_i64)),
            ("text_tokens", Value::from(5_i64)),
        ])
    );
    assert_eq!(
        usage["prompt_tokens_details"],
        value(&[
            ("cached_tokens", Value::from(4_i64)),
            ("cache_write_tokens", Value::from(2_i64)),
            ("cache_creation_tokens", Value::from(2_i64)),
        ])
    );
    assert_eq!(usage["prompt_cache_hit_tokens"], Value::from(4_i64));
}

fn numbered_tool_calls() {
    let sent = Value::Array(vec![
        value(&[
            ("index", Value::from(5_i64)),
            ("id", text("a")),
            ("function", value(&[("arguments", text("{"))])),
        ]),
        value(&[("function", value(&[("name", text("f"))]))]),
        value(&[
            ("id", Value::Null),
            ("function", value(&[("arguments", Value::Null)])),
        ]),
        value(&[("index", Value::Null), ("function", value(&[]))]),
        value(&[("custom", value(&[("name", text("n"))]))]),
    ]);
    let read = parse(&envelope(delta_choices(value(&[("tool_calls", sent)])))).unwrap();
    let calls = read.choices[0].delta.tool_calls.as_ref().unwrap();
    let indexes = calls.iter().map(|call| call.index).collect::<Vec<_>>();
    assert_eq!(indexes, [Some(5), Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(calls[0].kind.as_deref(), Some("function"));
    assert_eq!(calls[0].function.as_ref().unwrap().arguments, "{");
    assert_eq!(
        calls[1].function.as_ref().unwrap().name.as_deref(),
        Some("f")
    );
    assert_eq!(calls[1].function.as_ref().unwrap().arguments, "");
    assert_eq!(calls[2].function.as_ref().unwrap().arguments, "");
    assert!(calls[4].custom.is_some());
    assert_eq!(calls[4].kind, None);
    let written = serde_json::to_value(calls).unwrap();
    assert_eq!(written[0]["id"], text("a"));
    assert!(written[1].get("id").is_none());
    assert!(written[2]["id"].is_null());
}

fn invalid_tool_calls() {
    for call in [
        value(&[("id", text("a"))]),
        value(&[("type", text("custom"))]),
        value(&[
            ("type", text("custom")),
            ("function", value(&[("name", text("f"))])),
        ]),
    ] {
        let delta = value(&[("tool_calls", Value::Array(vec![call]))]);
        assert!(matches!(
            parse(&envelope(delta_choices(delta))),
            Err(StreamError::ToolCallPayload)
        ));
    }
}

fn dict_arguments() {
    let calls = Value::Array(vec![
        value(&[(
            "function",
            value(&[
                ("name", text("f")),
                ("arguments", value(&[("x", Value::from(1_i64))])),
            ]),
        )]),
        value(&[(
            "function",
            value(&[
                ("arguments", Value::Null),
                ("parameters", value(&[("y", Value::from(2_i64))])),
            ]),
        )]),
        value(&[(
            "function",
            value(&[("parameters", value(&[("z", Value::from(3_i64))]))]),
        )]),
        value(&[(
            "function",
            value(&[("arguments", text("t")), ("parameters", text("ignored"))]),
        )]),
        value(&[(
            "function",
            value(&[("arguments", Value::Null), ("parameters", text("ignored"))]),
        )]),
    ]);
    let read = parse(&envelope(delta_choices(value(&[("tool_calls", calls)])))).unwrap();
    let sent = read.choices[0].delta.tool_calls.as_ref().unwrap();
    let arguments = sent
        .iter()
        .map(|call| call.function.as_ref().unwrap().arguments.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        arguments,
        [r#"{"x":1}"#, r#"{"y":2}"#, r#"{"z":3}"#, "t", ""]
    );
    assert_eq!(sent[0].kind.as_deref(), Some("function"));
    non_text_arguments();
}

fn non_text_arguments() {
    let wrong_type = value(&[("function", value(&[("arguments", Value::from(5_i64))]))]);
    let delta = value(&[("tool_calls", Value::Array(vec![wrong_type]))]);
    assert!(matches!(
        parse(&envelope(delta_choices(delta))),
        Err(StreamError::Chunk(_))
    ));
}
