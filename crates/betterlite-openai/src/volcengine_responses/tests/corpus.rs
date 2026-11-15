use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::volcengine_responses::stream::decode_stream_event;

#[rstest]
fn replays_the_volcengine_stream_event_corpus(
    #[files("fuzz/corpus/volcengine_stream_event/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let decoded = decode_stream_event(&bytes);
    if name.starts_with("malformed") {
        assert!(decoded.is_err(), "{name} should be malformed");
    } else {
        assert!(decoded.is_ok(), "{name} should decode");
    }
}
