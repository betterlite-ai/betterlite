use std::fs;
use std::path::PathBuf;

use crate::azure_stream_errors::content_filter::decode_content_filter_error;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_stream_errors")
}

fn name_of(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn replays_the_azure_stream_errors_corpus() {
    let mut seen = 0_usize;
    for entry in fs::read_dir(corpus()).expect("the azure_stream_errors corpus exists") {
        let path = entry.expect("a readable corpus entry").path();
        if !path.is_file() {
            continue;
        }
        let name = name_of(&path);
        let bytes = fs::read(&path).expect("the seed is readable");
        let decoded = decode_content_filter_error(&bytes);
        seen = seen.saturating_add(1);
        if name.starts_with("malformed") || name.starts_with("truncated") {
            assert!(decoded.is_err(), "{name} must fail to parse");
        } else if name.starts_with("valid") {
            assert!(
                decoded.expect("a valid seed parses").is_some(),
                "{name} must be a content-filter error"
            );
        } else if name.starts_with("boundary") {
            decoded.expect("a boundary seed parses");
        }
    }
    assert!(seen > 0, "the corpus must hold seeds");
}
