use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use uuid::Uuid;

use crate::invoke_events::{DecoderClock, DecoderOptions};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const MODEL: &str = "bedrock/model";

pub(super) fn fixed_id() -> Uuid {
    Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef)
}

pub(super) fn fixed_now() -> i64 {
    NOW
}

pub(super) fn clock() -> DecoderClock {
    DecoderClock {
        next_id: fixed_id,
        now: fixed_now,
    }
}

pub(super) fn options(json_mode: Option<bool>, sync_stream: bool) -> DecoderOptions<'static> {
    DecoderOptions {
        model: MODEL,
        json_mode,
        sync_stream,
        clock: clock(),
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn text(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

pub(super) fn event_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
    let headers = vec![
        Header::new(":event-type".to_owned(), text(event_type)),
        Header::new(":message-type".to_owned(), text("event")),
    ];
    let message = Message::new_from_parts(headers, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).unwrap();
    out
}

pub(super) fn chunk_frame(source: &str) -> Vec<u8> {
    let payload = format!("{{\"bytes\":\"{}\"}}", STANDARD.encode(source));
    event_frame("chunk", payload.as_bytes())
}

pub(super) fn json(source: &str) -> betterlite_models::decisions::Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn request_headers() -> std::collections::BTreeMap<String, String> {
    std::collections::BTreeMap::from([
        (
            "content-type".to_owned(),
            "application/vnd.amazon.eventstream".to_owned(),
        ),
        ("x-amzn-requestid".to_owned(), "req-1".to_owned()),
    ])
}

pub(super) fn block_on<Future>(future: Future) -> Future::Output
where
    Future: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}
