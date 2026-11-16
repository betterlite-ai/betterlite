use std::fs;
use std::path::{Path, PathBuf};

use crate::image_edits::reply::decode_mai_edit_reply;

const NOW: i64 = 1_700_000_000;

#[test]
fn replays_the_foundry_image_edits_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_image_edits");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            let decoded = decode_mai_edit_reply(&bytes, 200, NOW);
            (decoded.is_ok() != name.starts_with("ok-")).then(|| name.to_owned())
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
