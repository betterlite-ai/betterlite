#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use bytes::BytesMut;

use crate::event_frames::decode_aws_event_frame;
use crate::event_frames::message::parse_message_from_event;
use crate::event_frames::shape::ResponseStreamShape;

fn outcome(bytes: &[u8]) -> &'static str {
    let shape = ResponseStreamShape::bedrock_runtime();
    let mut buffer = BytesMut::from(bytes);
    let mut events = 0_u32;
    loop {
        match decode_aws_event_frame(&mut buffer) {
            Err(_source) => return "rejected",
            Ok(None) if events == 0 => return "incomplete",
            Ok(None) => return "events",
            Ok(Some(event)) => {
                events = events.saturating_add(1);
                let _text = parse_message_from_event(Some(&shape), &event);
            }
        }
    }
}

#[test]
fn replays_the_bedrock_event_frames_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_event_frames");
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_stem().unwrap().to_str().unwrap();
            let category = name.rsplit('-').next().unwrap();
            assert_eq!(outcome(&bytes), category, "{name}");
            Ok::<(), io::Error>(())
        })
        .unwrap();
}
