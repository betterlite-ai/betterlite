use std::collections::BTreeMap;
use std::fmt::Display;

use aws_smithy_types::event_stream::HeaderValue;
use betterlite_bedrock::event_frames::beta::anthropic_beta_from_headers;
use betterlite_bedrock::event_frames::error::EventFrameError;
use betterlite_bedrock::event_frames::message::parse_message_from_event;
use betterlite_bedrock::event_frames::shape::ResponseStreamShape;
use betterlite_bedrock::event_frames::sse::ServerSentEvent;
use betterlite_bedrock::event_frames::tally::EventStreamTally;
use betterlite_bedrock::event_frames::{AwsEvent, decode_aws_event_frame};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use bytes::BytesMut;
use input::{Frames, Input, Sse, Stream};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod input;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Frames(frames) => frames_output(frames),
            Input::Stream(stream) => stream_output(stream),
            Input::Beta(beta) => Ok(json!(anthropic_beta_from_headers(&beta.headers))),
            Input::Sse(sse) => Ok(sse_output(sse)),
        }
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "EventFramesError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn decode_chunks(chunks: &[String]) -> Result<Vec<Vec<u8>>, Box<HelperError>> {
    chunks
        .iter()
        .map(|chunk| hex::decode(chunk).map_err(failure))
        .collect()
}

// The iter_bytes loop up to the point where each message is parsed.
fn walk<F>(
    chunks: &[Vec<u8>],
    tally: &mut EventStreamTally,
    mut on_event: F,
) -> Result<(), EventFrameError>
where
    F: FnMut(&AwsEvent) -> Result<(), EventFrameError>,
{
    let mut buffer = BytesMut::new();
    for chunk in chunks {
        buffer.extend_from_slice(chunk);
        tally.add_chunk(chunk);
        while let Some(event) = decode_aws_event_frame(&mut buffer)? {
            tally.add_event(&event);
            on_event(&event)?;
        }
    }
    Ok(())
}

fn failure_json(problem: &EventFrameError) -> Value {
    match problem {
        EventFrameError::Bedrock { status, error } => {
            json!({"kind": "bedrock", "status": status, "message": error.error.message})
        }
        EventFrameError::PreludeChecksum { .. }
        | EventFrameError::Frame(_)
        | EventFrameError::DuplicateHeader { .. }
        | EventFrameError::NotUtf8(_)
        | EventFrameError::NotJson(_)
        | EventFrameError::PayloadNotObject
        | EventFrameError::ChunkBytesNotText
        | EventFrameError::ChunkBase64(_)
        | EventFrameError::Header(_) => json!({"kind": "invalid", "status": null, "message": null}),
    }
}

fn header_json(value: &HeaderValue) -> Value {
    match value {
        HeaderValue::Bool(flag) => json!({"type": "bool", "value": flag}),
        HeaderValue::Byte(number) => json!({"type": "int", "value": number}),
        HeaderValue::Int16(number) => json!({"type": "int", "value": number}),
        HeaderValue::Int32(number) => json!({"type": "int", "value": number}),
        HeaderValue::Int64(number) => json!({"type": "int", "value": number}),
        HeaderValue::ByteArray(bytes) => json!({"type": "bytes", "value": hex::encode(bytes)}),
        HeaderValue::String(text) => json!({"type": "string", "value": text.as_str()}),
        HeaderValue::Timestamp(time) => json!({"type": "int", "value": time.to_millis().ok()}),
        HeaderValue::Uuid(id) => json!({"type": "bytes", "value": hex::encode(id.to_be_bytes())}),
        _ => json!({"type": "unknown", "value": null}),
    }
}

fn frames_output(input: &Frames) -> HelperResult {
    let chunks = decode_chunks(&input.chunks)?;
    let mut events = Vec::new();
    let outcome = walk(&chunks, &mut EventStreamTally::default(), |event| {
        let headers: BTreeMap<&String, Value> = event
            .headers
            .iter()
            .map(|(name, value)| (name, header_json(value)))
            .collect();
        events.push(json!({
            "headers": headers,
            "payload": hex::encode(&event.payload),
            "total_length": event.total_length,
        }));
        Ok(())
    });
    let failed = outcome.err().map(|error| failure_json(&error));
    Ok(json!({"events": events, "failure": failed}))
}

fn stream_output(input: &Stream) -> HelperResult {
    let chunks = decode_chunks(&input.chunks)?;
    let shape = input
        .shape_loaded
        .then(ResponseStreamShape::bedrock_runtime);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let mut tally = EventStreamTally::default();
    let mut texts = Vec::new();
    let mut outcome = walk(&chunks, &mut tally, |event| {
        texts.push(parse_message_from_event(shape.as_ref(), event)?);
        Ok(())
    });
    if outcome.is_ok()
        && let Some(undecoded) =
            tally.undecoded_stream_error(input.response_headers.as_ref(), &redactor)
    {
        outcome = Err(undecoded);
    }
    let failed = outcome.err().map(|error| failure_json(&error));
    Ok(json!({"texts": texts, "failure": failed}))
}

fn sse_output(input: &Sse) -> Value {
    let record = ServerSentEvent::new(
        input.event.clone(),
        input.data.clone(),
        input.id.clone(),
        input.retry,
    );
    let parsed = record.json::<Value>().map_or_else(
        |_| json!({"ok": false, "value": null}),
        |value| json!({"ok": true, "value": value}),
    );
    json!({
        "event": record.event,
        "data": record.data,
        "id": record.id,
        "retry": record.retry,
        "repr": record.repr(),
        "json": parsed,
    })
}
