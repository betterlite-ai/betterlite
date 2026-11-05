#![no_main]

use std::collections::BTreeMap;

use betterlite_bedrock::event_frames::beta::{BetaHeader, anthropic_beta_from_headers};
use betterlite_bedrock::event_frames::decode_aws_event_frame;
use betterlite_bedrock::event_frames::message::parse_message_from_event;
use betterlite_bedrock::event_frames::shape::ResponseStreamShape;
use betterlite_bedrock::event_frames::sse::ServerSentEvent;
use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let shape = ResponseStreamShape::bedrock_runtime();
    let mut buffer = BytesMut::from(bytes);
    while let Ok(Some(event)) = decode_aws_event_frame(&mut buffer) {
        let _text = parse_message_from_event(Some(&shape), &event);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let headers = BTreeMap::from([(
            "anthropic-beta".to_owned(),
            BetaHeader::Text(text.to_owned()),
        )]);
        let _betas = anthropic_beta_from_headers(&headers);
        let record = ServerSentEvent::new(None, Some(text.to_owned()), None, None);
        let _data = record.json::<Vec<u32>>();
    }
});
