use std::fs;
use std::path::{Path, PathBuf};

use super::super::decode::{ReplyFormat, decode_transcription_reply};
use crate::transcription::error::TranscriptionError;

fn category(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the seed name is UTF-8");
    name.split('-')
        .next()
        .expect("the seed has a category")
        .to_owned()
}

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).expect("the seed reads");
    let json = decode_transcription_reply(&bytes, Some("application/json"), ReplyFormat::Json);
    let text = decode_transcription_reply(&bytes, Some("text/plain"), ReplyFormat::Text);
    match category(path).as_str() {
        "valid" => json.is_ok(),
        "nonobject" => matches!(json, Err(TranscriptionError::NotObject)),
        "malformed" | "truncated" => matches!(json, Err(TranscriptionError::Json { .. })),
        "text" => text.is_ok() && matches!(json, Err(TranscriptionError::Json { .. })),
        // a boundary seed only has to come back; its JSON half may be either outcome
        "boundary" => text.is_ok(),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_transcription_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_transcription");
    let seeds: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    assert!(!seeds.is_empty(), "the corpus has seeds");
    assert_eq!(
        seeds
            .iter()
            .find(|path| !replays(path))
            .map(PathBuf::as_path),
        None
    );
}
