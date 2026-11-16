use std::fs;
use std::path::{Path, PathBuf};

use crate::mai_images::reply::decode_mai_image_reply;

#[test]
fn replays_the_foundry_mai_images_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_mai_images");
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
            let decoded = decode_mai_image_reply(&bytes, 200);
            (decoded.is_ok() != name.starts_with("ok-")).then(|| name.to_owned())
        })
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
