use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::stream_decode::error::DecodeError;
use crate::stream_decode::passthrough::{ConvertError, ConvertedChunk};
use crate::stream_decode::{DecodedChunk, TransportChunk};
use crate::stream_events::error::StreamEventError;

use super::fixture::{
    Outcome, both_drivers, bytes, convert, drive_sync, failure, generic, other_error,
    stream_failure, text, value_error,
};

fn kinds(outcomes: &[Outcome]) -> Vec<&'static str> {
    outcomes
        .iter()
        .map(|outcome| match outcome {
            Ok(DecodedChunk::Event(_)) => "event",
            Ok(DecodedChunk::Generic(_)) => "generic",
            Err(DecodeError::Receiving { .. }) => "receiving",
            Err(DecodeError::Other { .. }) => "other",
            Err(DecodeError::Parsing { .. }) => "parsing",
            Err(DecodeError::Event(StreamEventError::Anthropic(_))) => "anthropic",
            Err(DecodeError::Event(_)) => "shape",
        })
        .collect()
}

fn contents(outcomes: &[Outcome]) -> Vec<String> {
    let mut contents = Vec::new();
    for outcome in outcomes {
        if let Ok(DecodedChunk::Event(chunk)) = outcome {
            contents.push(chunk.choices[0].delta.content.clone());
        }
    }
    contents
}

fn delta(label: &str) -> String {
    format!(
        r#"data:{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{label}"}}}}"#
    )
}

fn assert_empty_generic(chunk: &GenericChunk) {
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.tool_use, None);
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
    assert_eq!(chunk.usage, None);
    assert_eq!(chunk.index, Some(0));
    assert_eq!(chunk.provider_specific_fields, None);
}

pub(super) fn split_event_data() {
    let steps = vec![
        Ok(text(&delta("a"))),
        Ok(text(&format!(
            "data:   {}",
            delta("b").trim_start_matches("data:")
        ))),
        Ok(text(&format!(
            "dataX{}",
            delta("c").trim_start_matches("data:")
        ))),
        Ok(text(r#"data: {"type":"pi"#)),
        Ok(text(r#"data: ng"}{"type":"message_stop"}"#)),
        Ok(text(
            r#"data: [1,2]{"type":"content_block_stop","index":0}"#,
        )),
        Ok(text(&delta("d"))),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        assert_eq!(
            kinds(outcomes),
            [
                "event", "event", "generic", "event", "event", "event", "event"
            ]
        );
        assert_eq!(contents(outcomes), ["a", "b", "", "", "", "d"]);
        assert_empty_generic(generic(&outcomes[2]));
    }
    // A nondict JSON value reaches the parser directly and fails there, without starting accumulation.
    let directly = drive_sync(vec![Ok(text("data: [1,2]")), Ok(text(&delta("e")))]);
    assert_eq!(kinds(&directly), ["shape", "event"]);
    assert_eq!(contents(&directly), ["e"]);
}

pub(super) fn split_unicode() {
    let steps = vec![
        Ok(text(&format!("xyz{}", delta("t")))),
        Ok(bytes(format!("xyz{}", delta("b")).as_bytes())),
        Ok(bytes(b"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\xe2\x82\xac\"}}")),
        Ok(bytes(b"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\xe2")),
        Ok(text(&delta("r"))),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        assert_eq!(
            kinds(outcomes),
            ["generic", "event", "event", "parsing", "event"]
        );
        assert_eq!(contents(outcomes), ["b", "\u{20ac}", "r"]);
        let message = failure(&outcomes[3]).to_string();
        assert!(message.starts_with("Error parsing chunk: "), "{message}");
        assert!(message.contains("Received chunk: "), "{message}");
    }
}

pub(super) fn multi_line_sse() {
    let steps = vec![
        Ok(text(&format!("event: x\n{}", delta("g")))),
        Ok(bytes(format!("event: x\n{}", delta("b")).as_bytes())),
        Ok(text(": keepalive\n\n")),
        Ok(text("")),
        Ok(text(&format!("{}\n{}", delta("m"), delta("n")))),
        Ok(text(&delta("s"))),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        assert_eq!(
            kinds(outcomes),
            ["generic", "event", "generic", "generic", "event"]
        );
        // Two data lines in one string are not joined or normalized; only the first value is parsed.
        assert_eq!(contents(outcomes), ["b", "m"]);
        assert_empty_generic(generic(&outcomes[0]));
        assert_empty_generic(generic(&outcomes[2]));
        assert_empty_generic(generic(&outcomes[3]));
    }
}

pub(super) fn done_marker() {
    let steps = vec![
        Ok(text(r#"data: {"type":"message_stop"}"#)),
        Ok(text(&delta("d"))),
        Ok(text("data: [DONE]")),
        Ok(text(&delta("e"))),
        Ok(text("data: [DONE]")),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        // [DONE] is not a terminator: it fails JSON and wedges the accumulator, so later events never come.
        assert_eq!(kinds(outcomes), ["event", "event"]);
        assert_eq!(contents(outcomes), ["", "d"]);
    }
    // A valid message_stop chunk is an ordinary event and does not end the iteration.
    let continuations = drive_sync(vec![
        Ok(text(r#"data: {"type":"message_stop"}"#)),
        Ok(text(&delta("f"))),
    ]);
    assert_eq!(kinds(&continuations), ["event", "event"]);
    assert_eq!(contents(&continuations), ["", "f"]);
}

pub(super) fn invalid_json_event() {
    let steps = vec![
        Ok(text("data: [1,2]")),
        Ok(text(&delta("i"))),
        Ok(text(r#"data: {"type":"content_block_delta""#)),
        Ok(text("data: [1,2]")),
        Ok(bytes(b"\xff\xfe")),
        value_error("bad read"),
        other_error("connection reset"),
        Ok(text(&delta("j"))),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        assert_eq!(
            kinds(outcomes),
            ["shape", "event", "parsing", "receiving", "other"]
        );
        assert_eq!(contents(outcomes), ["i"]);
    }
    let receiving = failure(&sync[3]).to_string();
    assert_eq!(receiving, "Error receiving chunk from stream: bad read");
    let other = failure(&sync[4]).to_string();
    assert_eq!(other, "connection reset");
    // The malformed JSON is silently left in the accumulator at end of stream.
    let truncated = drive_sync(vec![Ok(text(r#"data: {"type":"content_block_delta""#))]);
    assert!(truncated.is_empty());
    let accumulated = drive_sync(vec![
        Ok(text(r#"data: {"type":"pi"#)),
        Ok(text(r#"data: ng""#)),
    ]);
    assert!(accumulated.is_empty());
}

pub(super) fn native_error_event() {
    let steps = vec![Ok(text(
        r#"data: {"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#,
    ))];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        assert_eq!(kinds(outcomes), ["anthropic"]);
        let StreamEventError::Anthropic(native) = stream_failure(failure(&outcomes[0])) else {
            panic!("a native error keeps its own class");
        };
        assert_eq!(native.status_code, 500);
        assert_eq!(native.message, "slow");
    }
    // The same native error reached through accumulation is not swallowed by the JSON ValueError path either.
    let accumulated = drive_sync(vec![
        Ok(text(r#"data: {"type":"err"#)),
        Ok(text(
            r#"data:or","error":{"type":"rate_limit_error","message":"slow"}}"#,
        )),
    ]);
    assert_eq!(kinds(&accumulated), ["anthropic"]);
    let StreamEventError::Anthropic(native) = stream_failure(failure(&accumulated[0])) else {
        panic!("a native error keeps its own class");
    };
    assert_eq!(native.status_code, 500);
}

pub(super) fn truncated_final_event() {
    let steps = vec![
        Ok(text(r#"data: {"type":"ping"}{"type":"message_stop"}"#)),
        Ok(text(&format!(
            r#"data: {{"type":"content_block_stop","index":0}}{}"#,
            delta("t").trim_start_matches("data:")
        ))),
    ];
    let (sync, asynchronous) = both_drivers(steps);
    for outcomes in [&sync, &asynchronous] {
        // One buffered dict per end-of-stream pull, and the trailing junk invents no error or finish chunk.
        assert_eq!(kinds(outcomes), ["event", "event", "event", "event"]);
        assert_eq!(contents(outcomes), ["", "", "", "t"]);
    }
    let truncated = drive_sync(vec![
        Ok(text(r#"data: {"type":"ping"}"#)),
        Ok(text(r#"data: {"type":"message_stop"#)),
    ]);
    assert_eq!(kinds(&truncated), ["event"]);
    let leftover = drive_sync(vec![Ok(text(r#"data: {"type":"ping"} junk"#))]);
    assert_eq!(kinds(&leftover), ["event"]);
    assert_eq!(contents(&leftover), [""]);
}

fn passthrough_chunks() -> Vec<TransportChunk> {
    vec![
        text("hello"),
        bytes(b"hello"),
        text(&format!("xyz{}", delta("p"))),
        bytes(format!("data: {}", delta("q").trim_start_matches("data:")).as_bytes()),
        text("data: nope"),
        text("data: [1]"),
        text(&format!(
            "{}{}",
            delta("r"),
            delta("s").trim_start_matches("data:")
        )),
        text("dataX u"),
        text(""),
        bytes(b"\xff"),
    ]
}

pub(super) fn passthrough_chunk_conversion() {
    let conversions = convert(passthrough_chunks());
    let kinds: Vec<&'static str> = conversions
        .iter()
        .map(|conversion| match conversion {
            Ok(ConvertedChunk::Event(_)) => "event",
            Ok(ConvertedChunk::Default { .. }) => "default",
            Err(ConvertError::Utf8(_)) => "utf8",
            Err(ConvertError::Json(_)) => "json",
            Err(ConvertError::Event(_)) => "event-error",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "default",
            "default",
            "event",
            "event",
            "json",
            "event-error",
            "json",
            "default",
            "default",
            "utf8"
        ]
    );
    let Ok(ConvertedChunk::Default { id }) = &conversions[0] else {
        panic!("a chunk with no data: line keeps the state id");
    };
    assert_eq!(id, super::fixture::RESPONSE_ID);
    let Ok(ConvertedChunk::Event(event)) = &conversions[2] else {
        panic!("the scanned text chunk parses into an event");
    };
    assert_eq!(event.choices[0].delta.content, "p");
}
