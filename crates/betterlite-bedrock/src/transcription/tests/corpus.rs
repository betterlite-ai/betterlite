use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::transcription::reply::decode_transcription_reply;

const EXPECTED_TEXTS: [(&str, &str); 3] = [
    ("valid_two_blocks.json", "hello world"),
    ("valid_empty_content.json", ""),
    ("boundary_unicode_text.json", "h\u{e9}llo \u{1f3a4}"),
];

#[rstest]
fn replays_the_bedrock_transcription_reply_corpus(
    #[files("fuzz/corpus/bedrock_transcription_reply/*")] path: PathBuf,
) {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let decodes = !name.starts_with("malformed") && !name.starts_with("truncated");
    assert_eq!(
        decode_transcription_reply(&bytes).is_ok(),
        decodes,
        "{name}"
    );
    if let Some((_, expected)) = EXPECTED_TEXTS.iter().find(|(file, _)| *file == name) {
        assert_eq!(decode_transcription_reply(&bytes).unwrap().text, *expected);
    }
}
