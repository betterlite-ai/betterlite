use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::passthrough::transformation::PassthroughStreamCollector;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use super::super::CollectorClock;
use super::super::collector::BedrockEventStreamCollector;
use super::super::record::{DecodedChunk, EventChunkDecoder, StreamLogError};
use super::{TestAssembler, TestLogging, clock, fixtures, json};

const FAIL_PAYLOAD: &[u8] = br#"{"fail":true}"#;
const ASSEMBLED: &str = r#"{"id":"assembled"}"#;

fn event_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut headers = Vec::new();
    let pairs = [
        (":event-type", event_type),
        (":content-type", "application/json"),
        (":message-type", "event"),
    ];
    for (name, value) in pairs {
        headers.push(u8::try_from(name.len()).unwrap());
        headers.extend(name.bytes());
        headers.push(7);
        headers.extend(u16::try_from(value.len()).unwrap().to_be_bytes());
        headers.extend(value.bytes());
    }
    let total = headers
        .len()
        .saturating_add(payload.len())
        .saturating_add(16);
    let mut bytes = Vec::with_capacity(total);
    bytes.extend(u32::try_from(total).unwrap().to_be_bytes());
    bytes.extend(u32::try_from(headers.len()).unwrap().to_be_bytes());
    let mut prelude = crc32fast::Hasher::new();
    prelude.update(&bytes);
    bytes.extend(prelude.finalize().to_be_bytes());
    bytes.extend(&headers);
    bytes.extend(payload);
    let mut message = crc32fast::Hasher::new();
    message.update(&bytes);
    bytes.extend(message.finalize().to_be_bytes());
    bytes
}

fn chunk_frame(message: &str) -> Vec<u8> {
    let encoded = STANDARD.encode(message.as_bytes());
    let payload = format!("{{\"bytes\":\"{encoded}\"}}");
    event_frame("chunk", payload.as_bytes())
}

fn hello_frame() -> Vec<u8> {
    chunk_frame(r#"{"chunk":{"id":"c1","model":"m","choices":[{"delta":{"content":"hi"}}]}}"#)
}

struct FixtureDecoder;

impl EventChunkDecoder for FixtureDecoder {
    fn decode(
        &mut self,
        chunk: &Json,
        _clock: CollectorClock,
    ) -> Result<DecodedChunk, StreamLogError> {
        let Json::Object(fields) = chunk else {
            return Ok(DecodedChunk::Ignored);
        };
        if let Some(spec) = fields.get("chunk") {
            let text = serde_json::to_string(spec).map_err(StreamLogError::Encode)?;
            return serde_json::from_str::<StreamChunk>(&text)
                .map(|parsed| DecodedChunk::Chunk(Box::new(parsed)))
                .map_err(StreamLogError::Decode);
        }
        if fields.contains_key("text") {
            return Ok(DecodedChunk::Generic(chunk.clone()));
        }
        if fields.contains_key("fail") {
            return Err(StreamLogError::Decoder {
                message: "fixture decoder failure".to_owned(),
            });
        }
        Ok(DecodedChunk::Ignored)
    }
}

fn collector<'plan>(
    decoder: Option<Box<dyn EventChunkDecoder + 'plan>>,
    assembler: &'plan TestAssembler,
) -> BedrockEventStreamCollector<'plan> {
    BedrockEventStreamCollector::new(decoder, assembler, clock())
}

fn fragments_and_generic_records() {
    let (_, _, assembler) = fixtures();
    let mut stream = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let frame = hello_frame();
    let (first, second) = frame.split_at(frame.len().div_euclid(2));
    stream.feed(first).unwrap();
    stream.feed(second).unwrap();
    let known =
        br#"{"text":"g","is_finished":false,"finish_reason":"","usage":{"total_tokens":3}}"#;
    stream.feed(&event_frame("other", known)).unwrap();
    let unknown = br#"{"text":"x","unexpected":1}"#;
    stream.feed(&event_frame("other", unknown)).unwrap();
    stream.feed(&event_frame("other", b"")).unwrap();
    let logged = stream.build_logged_response(&mut TestLogging).unwrap();
    assert_eq!(logged, Some(json(ASSEMBLED)));
    assert_eq!(*assembler.calls.lock().unwrap(), 1);
    let chunks = assembler.chunks.lock().unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].id, "c1");
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("hi"));
    assert_eq!(chunks[1].choices[0].delta.content.as_deref(), Some("g"));
    assert_eq!(chunks[1].id.len(), 36);
}

fn disabled_and_empty_collectors() {
    let (_, _, assembler) = fixtures();
    let mut disabled = collector(None, &assembler);
    disabled.feed(b"not a frame").unwrap();
    disabled.feed(&hello_frame()).unwrap();
    let skipped = disabled.build_logged_response(&mut TestLogging).unwrap();
    assert_eq!(skipped, None);
    let mut empty = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let nothing = empty.build_logged_response(&mut TestLogging).unwrap();
    assert_eq!(nothing, None);
    assert_eq!(*assembler.calls.lock().unwrap(), 0);
}

fn decoder_and_frame_errors_propagate() {
    let (_, _, assembler) = fixtures();
    let mut failing = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let failure = failing
        .feed(&event_frame("other", FAIL_PAYLOAD))
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the stream decoder failed: fixture decoder failure"
    );
    let mut malformed = collector(Some(Box::new(FixtureDecoder)), &assembler);
    malformed.feed(b"\x00\x00\x00\x10garbage").unwrap();
    let mut broken = event_frame("other", b"{}");
    *broken.last_mut().unwrap() ^= 0xff;
    let frame_error = malformed.feed(&broken).unwrap_err();
    assert!(matches!(frame_error, StreamLogError::Frame(_)));
}

fn trait_collector_keeps_the_failure() {
    let (_, _, assembler) = fixtures();
    let mut stream = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let dynamic: &mut dyn PassthroughStreamCollector<Logged = Json> = &mut stream;
    dynamic.add(&hello_frame());
    let logged = dynamic.build_logged_response(&mut TestLogging).unwrap();
    assert_eq!(logged, Some(json(ASSEMBLED)));
    dynamic.add(&event_frame("other", FAIL_PAYLOAD));
    let after = dynamic.build_logged_response(&mut TestLogging).unwrap();
    assert_eq!(after, None);
}

pub(super) fn event_stream_collector() {
    fragments_and_generic_records();
    disabled_and_empty_collectors();
    decoder_and_frame_errors_propagate();
    trait_collector_keeps_the_failure();
}
