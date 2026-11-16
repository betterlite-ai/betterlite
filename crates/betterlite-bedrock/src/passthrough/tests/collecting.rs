use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::passthrough::transformation::PassthroughStreamCollector;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use super::super::CollectorClock;
use super::super::collector::BedrockEventStreamCollector;
use super::super::record::{DecodedChunk, EventChunkDecoder, StreamLogError};
use super::{TestAssembler, TestLogging, clock, fixtures, json};

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
    let total = 12 + headers.len() + payload.len() + 4;
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
                .map(DecodedChunk::Chunk)
                .map_err(StreamLogError::Decode);
        }
        if let Some(generic) = fields.get("generic") {
            return Ok(DecodedChunk::Generic(generic.clone()));
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

pub(super) fn event_stream_collector() {
    let (_, _, assembler) = fixtures();
    let mut logging = TestLogging;
    let mut stream = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let frame =
        chunk_frame(r#"{"chunk":{"id":"c1","model":"m","choices":[{"delta":{"content":"hi"}}]}}"#);
    let split = frame.len() / 2;
    stream.feed(&frame[..split]).unwrap();
    stream.feed(&frame[split..]).unwrap();
    let known = br#"{"text":"g","is_finished":false,"finish_reason":""}"#;
    stream.feed(&event_frame("other", known)).unwrap();
    let unknown = br#"{"text":"x","unexpected":1}"#;
    stream.feed(&event_frame("other", unknown)).unwrap();
    stream.feed(&event_frame("other", b"")).unwrap();
    let logged = stream.build_logged_response(&mut logging).unwrap();
    assert_eq!(logged, Some(json(r#"{"id":"assembled"}"#)));
    {
        let chunks = assembler.chunks.lock().unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "c1");
        assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("hi"));
        assert_eq!(chunks[1].choices[0].delta.content.as_deref(), Some("g"));
        assert_eq!(chunks[1].id.len(), 36);
    }
    assert_eq!(*assembler.calls.lock().unwrap(), 1);

    let mut disabled = collector(None, &assembler);
    disabled.feed(b"not a frame").unwrap();
    disabled.feed(&frame).unwrap();
    assert_eq!(disabled.build_logged_response(&mut logging).unwrap(), None);
    assert_eq!(*assembler.calls.lock().unwrap(), 1);

    let mut empty = collector(Some(Box::new(FixtureDecoder)), &assembler);
    assert_eq!(empty.build_logged_response(&mut logging).unwrap(), None);

    let mut failing = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let failure = failing
        .feed(&event_frame("other", br#"{"fail":true}"#))
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "the stream decoder failed: fixture decoder failure"
    );

    let mut malformed = collector(Some(Box::new(FixtureDecoder)), &assembler);
    malformed.feed(b"\x00\x00\x00\x10garbage").unwrap();
    let mut broken = event_frame("other", b"{}");
    let last = broken.len() - 1;
    broken[last] ^= 0xff;
    assert!(matches!(
        malformed.feed(&broken).unwrap_err(),
        StreamLogError::Frame(_)
    ));

    let mut dynamic_stream = collector(Some(Box::new(FixtureDecoder)), &assembler);
    let dynamic: &mut dyn PassthroughStreamCollector<Logged = Json> = &mut dynamic_stream;
    dynamic.add(&frame);
    assert_eq!(
        dynamic.build_logged_response(&mut logging).unwrap(),
        Some(json(r#"{"id":"assembled"}"#))
    );
    dynamic.add(&event_frame("other", br#"{"fail":true}"#));
    assert_eq!(dynamic.build_logged_response(&mut logging).unwrap(), None);
}
