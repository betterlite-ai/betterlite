#![cfg(test)]

use serde_json::Value;

use super::support::{fixed_clock, nil_id, text, value};
use crate::chat_choices::stream::{
    CHUNK_OBJECT, StreamError, chunk_parser, extract_error_from_chunk,
    map_reasoning_to_reasoning_content,
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

    let mut choices = vec![value(&[("delta", value(&[("reasoning", text("r"))]))])];
    map_reasoning_to_reasoning_content(&mut choices).unwrap();
    assert_eq!(choices[0]["delta"]["reasoning_content"], text("r"));
    assert!(choices[0]["delta"].get("reasoning").is_none());
    assert!(matches!(
        map_reasoning_to_reasoning_content(&mut [Value::from(5_i64)]),
        Err(StreamError::ChoiceNotObject)
    ));
    assert!(matches!(
        map_reasoning_to_reasoning_content(&mut [value(&[("delta", Value::from(5_i64))])]),
        Err(StreamError::DeltaNotObject)
    ));
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

pub(super) fn embedded_stream_error() {
    assert_eq!(
        extract_error_from_chunk(&error_chunk(value(&[
            ("message", text("boom")),
            ("code", Value::from(400_i64)),
        ]))),
        Some((String::from("boom"), 400))
    );
    assert_eq!(
        extract_error_from_chunk(&error_chunk(value(&[
            ("message", text("x")),
            ("code", Value::from(599_i64)),
        ]))),
        Some((String::from("x"), 599))
    );
    status_bounds();
    ignored_errors();
    assert_eq!(
        error_status(&error_chunk(value(&[
            ("message", text("boom")),
            ("code", Value::from(400_i64)),
        ]))),
        Some(400)
    );
    assert_eq!(error_status(&error_chunk(text("plain"))), Some(500));
    false_errors_are_ignored();
}

fn false_errors_are_ignored() {
    let false_error = parse(&value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("error", Value::Bool(false)),
    ]))
    .unwrap();
    assert_eq!(false_error.id, "c");
    let zero_error = parse(&value(&[
        ("id", text("c")),
        ("created", Value::from(1_i64)),
        ("model", text("m")),
        ("error", Value::from(0_i64)),
    ]))
    .unwrap();
    assert_eq!(zero_error.id, "c");
}

fn status_bounds() {
    for (code, expected) in [(600_i64, 500_u16), (399, 500), (500, 500)] {
        let error = error_chunk(value(&[
            ("message", text("x")),
            ("code", Value::from(code)),
        ]));
        assert_eq!(error_status(&error), Some(expected));
    }
    let text_code = error_chunk(value(&[("message", text("x")), ("code", text("400"))]));
    assert_eq!(error_status(&text_code), Some(500));
    let float_code = error_chunk(value(&[
        ("message", text("x")),
        ("code", Value::from(400.0_f64)),
    ]));
    assert_eq!(error_status(&float_code), Some(500));
    assert_eq!(
        extract_error_from_chunk(&error_chunk(text("plain"))),
        Some((String::from("plain"), 500))
    );
    assert_eq!(
        extract_error_from_chunk(&error_chunk(Value::Bool(true))).map(|(_, status)| status),
        Some(500)
    );
}

fn ignored_errors() {
    for ignored in [
        Value::Null,
        Value::Bool(false),
        Value::from(0_i64),
        text(""),
        Value::Array(Vec::new()),
        value(&[]),
    ] {
        assert_eq!(extract_error_from_chunk(&error_chunk(ignored)), None);
    }
    assert_eq!(extract_error_from_chunk(&value(&[("id", text("c"))])), None);
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
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (1, 2, 3)
    );
    assert!(usage.prompt_tokens_details.is_none());
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
        ("usage", value(&[("prompt_tokens", Value::Null)])),
    ]))
    .unwrap();
    let usage = zeroed.usage.as_ref().unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ),
        (0, 0, 0)
    );
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
        Err(StreamError::ChoicesNotList)
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
        Err(StreamError::ChoiceNotObject)
    ));
}
