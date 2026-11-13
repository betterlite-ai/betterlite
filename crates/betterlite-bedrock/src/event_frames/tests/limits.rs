#![cfg(test)]

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use bytes::BytesMut;

use super::frames::{bedrock_parts, flip, kind};
use crate::event_frames::tally::EventStreamTally;
use crate::event_frames::{MAX_FRAME_BYTES, MAX_HEADER_BYTES, decode_aws_event_frame};

fn prelude(total: u32, headers: u32) -> BytesMut {
    let mut head = Vec::new();
    head.extend(total.to_be_bytes());
    head.extend(headers.to_be_bytes());
    let crc = crc32fast::hash(&head);
    head.extend(crc.to_be_bytes());
    BytesMut::from(head.as_slice())
}

#[test]
fn event_frames_limits_and_redaction() {
    let mut huge = prelude(MAX_FRAME_BYTES + 1, 0);
    let (status, text) = bedrock_parts(decode_aws_event_frame(&mut huge).unwrap_err());
    assert_eq!(status, 502);
    assert!(text.contains("exceeds"), "{text}");
    assert_eq!(huge.len(), 12);

    let mut wide = prelude(MAX_FRAME_BYTES, MAX_HEADER_BYTES + 1);
    let (wide_status, wide_text) = bedrock_parts(decode_aws_event_frame(&mut wide).unwrap_err());
    assert_eq!(wide_status, 502);
    assert!(wide_text.contains("headers"), "{wide_text}");

    let mut at_limit = prelude(MAX_FRAME_BYTES, MAX_HEADER_BYTES);
    assert!(decode_aws_event_frame(&mut at_limit).unwrap().is_none());
    assert!(at_limit.capacity() < 1024);

    let mut forged = prelude(MAX_FRAME_BYTES + 1, 0);
    flip(&mut forged, 11);
    assert_eq!(
        kind(&decode_aws_event_frame(&mut forged).unwrap_err()),
        "PreludeChecksum"
    );

    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut tally = EventStreamTally::default();
    tally.add_chunk(
        b"{\"key\":\"AKIAIOSFODNN7EXAMPLE\",\"token\":\"sk-abcdefghijklmnopqrstuvwxyz123456\"}",
    );
    let error = tally.undecoded_stream_error(None, &redactor).unwrap();
    let (_, message) = bedrock_parts(error);
    assert!(!message.contains("AKIAIOSFODNN7EXAMPLE"), "{message}");
    assert!(
        !message.contains("sk-abcdefghijklmnopqrstuvwxyz123456"),
        "{message}"
    );
    assert!(message.contains("first bytes="), "{message}");
}
