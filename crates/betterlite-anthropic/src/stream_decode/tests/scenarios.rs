use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;

use crate::stream_decode::error::{DecodeError, ReceiveError};
use crate::stream_decode::passthrough::{ConvertError, ConvertedChunk};
use crate::stream_decode::{DecodedChunk, TransportChunk};
use crate::stream_events::error::StreamEventError;

use super::fixture::{
    Outcome, SourceFailure, convert, drive_async, drive_sync, failure, generic, stream_failure,
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
        Ok(TransportChunk::Text(delta("a"))),
        Ok(TransportChunk::Text(format!(
            "data:   {}",
            delta("b").trim_start_matches("data:")
        ))),
        Ok(TransportChunk::Text(format!(
            "dataX{}",
            delta("c").trim_start_matches("data:")
        ))),
        Ok(TransportChunk::Text(r#"data: {"type":"pi"#.to_owned())),
        Ok(TransportChunk::Text(
            r#"data: ng"}{"type":"message_stop"}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(
            r#"data: [1,2]{"type":"content_block_stop","index":0}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(delta("d"))),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
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
    let directly = drive_sync(vec![
        Ok(TransportChunk::Text("data: [1,2]".to_owned())),
        Ok(TransportChunk::Text(delta("e"))),
    ]);
    assert_eq!(kinds(&directly), ["shape", "event"]);
    assert_eq!(contents(&directly), ["e"]);
    // The five-character strip keeps what follows data:, so a split inside a JSON string keeps its space.
    let inside_string = drive_sync(vec![
        Ok(TransportChunk::Text(
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"#
                .to_owned(),
        )),
        Ok(TransportChunk::Text(r#"data: b"}}"#.to_owned())),
    ]);
    assert_eq!(kinds(&inside_string), ["event"]);
    assert_eq!(contents(&inside_string), ["a b"]);
    // A text chunk is not scanned for data:, so one leading space keeps it out of the data: branch.
    let leading_space = drive_sync(vec![Ok(TransportChunk::Text(
        r#" data:{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"x"}}"#
            .to_owned(),
    ))]);
    assert_eq!(kinds(&leading_space), ["generic"]);
    assert_empty_generic(generic(&leading_space[0]));
}

pub(super) fn split_unicode() {
    let steps = vec![
        Ok(TransportChunk::Text(format!("xyz{}", delta("t")))),
        Ok(TransportChunk::Bytes(
            format!("xyz{}", delta("b")).into_bytes(),
        )),
        Ok(TransportChunk::Bytes(
            b"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\xe2\x82\xac\"}}".to_vec(),
        )),
        Ok(TransportChunk::Bytes(
            b"data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"\xe2".to_vec(),
        )),
        Ok(TransportChunk::Text(delta("r"))),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
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
        Ok(TransportChunk::Text(format!("event: x\n{}", delta("g")))),
        Ok(TransportChunk::Bytes(
            format!("event: x\n{}", delta("b")).into_bytes(),
        )),
        Ok(TransportChunk::Text(": keepalive\n\n".to_owned())),
        Ok(TransportChunk::Text(String::new())),
        Ok(TransportChunk::Text(format!(
            "{}\n{}",
            delta("m"),
            delta("n")
        ))),
        Ok(TransportChunk::Text(delta("s"))),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
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
        Ok(TransportChunk::Text(
            r#"data: {"type":"message_stop"}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(delta("d"))),
        Ok(TransportChunk::Text("data: [DONE]".to_owned())),
        Ok(TransportChunk::Text(delta("e"))),
        Ok(TransportChunk::Text("data: [DONE]".to_owned())),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
    for outcomes in [&sync, &asynchronous] {
        // [DONE] is not a terminator: it fails JSON and wedges the accumulator, so later events never come.
        assert_eq!(kinds(outcomes), ["event", "event"]);
        assert_eq!(contents(outcomes), ["", "d"]);
    }
    // A valid message_stop chunk is an ordinary event and does not end the iteration.
    let continuations = drive_sync(vec![
        Ok(TransportChunk::Text(
            r#"data: {"type":"message_stop"}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(delta("f"))),
    ]);
    assert_eq!(kinds(&continuations), ["event", "event"]);
    assert_eq!(contents(&continuations), ["", "f"]);
}

pub(super) fn invalid_json_event() {
    let steps = vec![
        Ok(TransportChunk::Text("data: [1,2]".to_owned())),
        Ok(TransportChunk::Text(delta("i"))),
        Ok(TransportChunk::Text(
            r#"data: {"type":"content_block_delta""#.to_owned(),
        )),
        Ok(TransportChunk::Text("data: [1,2]".to_owned())),
        Ok(TransportChunk::Bytes(b"\xff\xfe".to_vec())),
        Err(ReceiveError::Value {
            source: SourceFailure {
                message: "bad read".to_owned(),
            },
        }),
        Err(ReceiveError::Other {
            source: SourceFailure {
                message: "connection reset".to_owned(),
            },
        }),
        Ok(TransportChunk::Text(delta("j"))),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
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
    let truncated = drive_sync(vec![Ok(TransportChunk::Text(
        r#"data: {"type":"content_block_delta""#.to_owned(),
    ))]);
    assert!(truncated.is_empty());
    let accumulated = drive_sync(vec![
        Ok(TransportChunk::Text(r#"data: {"type":"pi"#.to_owned())),
        Ok(TransportChunk::Text(r#"data: ng""#.to_owned())),
    ]);
    assert!(accumulated.is_empty());
}

pub(super) fn native_error_event() {
    let steps = vec![Ok(TransportChunk::Text(
        r#"data: {"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#.to_owned(),
    ))];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
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
        Ok(TransportChunk::Text(r#"data: {"type":"err"#.to_owned())),
        Ok(TransportChunk::Text(
            r#"data:or","error":{"type":"rate_limit_error","message":"slow"}}"#.to_owned(),
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
        Ok(TransportChunk::Text(
            r#"data: {"type":"ping"}{"type":"message_stop"}"#.to_owned(),
        )),
        Ok(TransportChunk::Text(format!(
            r#"data: {{"type":"content_block_stop","index":0}}{}"#,
            delta("t").trim_start_matches("data:")
        ))),
    ];
    let (sync, asynchronous) = (drive_sync(steps.clone()), drive_async(steps));
    for outcomes in [&sync, &asynchronous] {
        // One buffered dict per end-of-stream pull, and the trailing junk invents no error or finish chunk.
        assert_eq!(kinds(outcomes), ["event", "event", "event", "event"]);
        assert_eq!(contents(outcomes), ["", "", "", "t"]);
    }
    let truncated = drive_sync(vec![
        Ok(TransportChunk::Text(r#"data: {"type":"ping"}"#.to_owned())),
        Ok(TransportChunk::Text(
            r#"data: {"type":"message_stop"#.to_owned(),
        )),
    ]);
    assert_eq!(kinds(&truncated), ["event"]);
    let leftover = drive_sync(vec![Ok(TransportChunk::Text(
        r#"data: {"type":"ping"} junk"#.to_owned(),
    ))]);
    assert_eq!(kinds(&leftover), ["event"]);
    assert_eq!(contents(&leftover), [""]);
}

fn passthrough_chunks() -> Vec<TransportChunk> {
    vec![
        TransportChunk::Text("hello".to_owned()),
        TransportChunk::Bytes(b"hello".to_vec()),
        TransportChunk::Text(format!("xyz{}", delta("p"))),
        TransportChunk::Bytes(
            format!("data: {}", delta("q").trim_start_matches("data:")).into_bytes(),
        ),
        TransportChunk::Text("data: nope".to_owned()),
        TransportChunk::Text("data: [1]".to_owned()),
        TransportChunk::Text(format!(
            "{}{}",
            delta("r"),
            delta("s").trim_start_matches("data:")
        )),
        TransportChunk::Text("dataX u".to_owned()),
        TransportChunk::Text(String::new()),
        TransportChunk::Bytes(b"\xff".to_vec()),
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
