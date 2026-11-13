use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::frames::PredictionFrames;
use crate::legacy_prediction::records::{
    ChunkKind, LegacyChunk, PREDICTION_BODY_LIMIT, chat_reply, decode_legacy_prediction, text_reply,
};
use crate::legacy_prediction::tensor::chunk_of;

#[test]
fn legacy_prediction_decodes_reply_shapes() {
    let reply = decode_legacy_prediction(
        br#"{"predictions":[{"content":"hello"}],"usageMetadata":{"promptTokenCount":3}}"#,
    )
    .expect("a text reply");
    let text = text_reply(&reply).expect("the text");
    assert_eq!(text.text, "hello");
    assert_eq!(
        reply
            .meta
            .usage_metadata
            .expect("usage metadata")
            .prompt_token_count,
        Some(3)
    );
}

#[test]
fn legacy_prediction_chat_requires_safety() {
    let reply = decode_legacy_prediction(br#"{"predictions":[{"candidates":[{"content":"hi"}]}]}"#)
        .expect("a body");
    assert!(matches!(
        chat_reply(&reply),
        Err(PredictionError::WrongFieldType {
            field: "safetyAttributes"
        })
    ));
}

#[test]
fn legacy_prediction_chat_reads_candidates() {
    let reply = decode_legacy_prediction(
        br#"{"predictions":[{"candidates":[{"content":"hi","finishReason":"MAX_TOKENS"}],"safetyAttributes":[{"blocked":false}]}]}"#,
    )
    .expect("a body");
    let text = chat_reply(&reply).expect("the chat reply");
    assert_eq!(text.text, "hi");
    assert_eq!(
        text.meta
            .candidates
            .expect("candidates")
            .first()
            .and_then(|candidate| candidate.finish_reason.as_deref()),
        Some("MAX_TOKENS")
    );
}

#[test]
fn legacy_prediction_rejects_a_large_body() {
    let too_large = vec![b' '; super::records::PREDICTION_BODY_LIMIT.saturating_add(1)];
    assert!(matches!(
        decode_legacy_prediction(&too_large),
        Err(PredictionError::TooLarge { .. })
    ));
}

#[test]
fn legacy_prediction_frames_split_across_reads() {
    let mut frames = PredictionFrames::default();
    assert!(frames.push(b"[").expect("the open").is_empty());
    assert!(
        frames
            .push(br#"{"outputs":[{"structVal":{"content":{"stringVal":["He"#)
            .expect("a partial object")
            .is_empty()
    );
    let ready = frames.push(br#"llo"]}}}]}"#).expect("the object close");
    assert_eq!(ready.len(), 1);
    assert!(frames.push(b"]").expect("the array close").is_empty());
    assert!(frames.finish().is_ok());
    let chunk = chunk_of(ChunkKind::Text, &ready[0]).expect("a chunk");
    assert_eq!(
        chunk,
        LegacyChunk {
            text: String::from("Hello")
        }
    );
}

#[test]
fn legacy_prediction_frames_reject_unfinished() {
    let mut frames = PredictionFrames::default();
    let ready = frames.push(br#"[{"outputs":[]}"#).expect("an object");
    assert_eq!(ready, vec![br#"{"outputs":[]}"#.to_vec()]);
    assert!(matches!(
        frames.finish(),
        Err(PredictionError::StreamUnfinished)
    ));
    assert!(matches!(
        PredictionFrames::default().push(b"{}"),
        Err(PredictionError::StreamShape)
    ));
}

#[test]
fn replays_the_legacy_prediction_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/legacy_prediction");
    let files: Vec<_> = fs::read_dir(corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!files.is_empty());
    files
        .iter()
        .try_for_each(|path| {
            let bytes = fs::read(path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("a UTF-8 seed name");
            let decoded = decode_legacy_prediction(&bytes);
            if name.starts_with("valid") {
                decoded.expect("this seed is a prediction body");
            } else {
                assert!(decoded.is_err(), "{name}");
            }
            Ok::<(), std::io::Error>(())
        })
        .expect("readable corpus");
}
