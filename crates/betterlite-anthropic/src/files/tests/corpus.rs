use std::fs;
use std::path::Path;

use super::support::NOW;
use crate::files::wire::decode_anthropic_file;

#[test]
fn replays_the_anthropic_files_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_files");
    let sources: Vec<(String, Vec<u8>)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect();
    assert!(sources.len() >= 4);
    let categories: Vec<(String, bool)> = sources
        .iter()
        .map(|(name, bytes)| {
            let accepted = decode_anthropic_file(bytes, NOW).is_ok();
            let expected = matches!(name.as_str(), "valid.json" | "boundary.json");
            (name.clone(), accepted == expected)
        })
        .collect();
    assert!(
        categories.iter().all(|(_, matches)| *matches),
        "{categories:?}"
    );
    let (_, valid) = sources
        .iter()
        .find(|(name, _)| name == "valid.json")
        .unwrap();
    let file = decode_anthropic_file(valid, NOW).unwrap();
    assert_eq!(file.id, "file-1");
    assert_eq!(file.bytes, 123);
    assert_eq!(file.created_at, 1_735_689_600);
    assert_eq!(file.filename, "doc.pdf");
}
