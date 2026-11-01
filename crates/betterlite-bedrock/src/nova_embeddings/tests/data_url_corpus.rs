#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;
use std::str;

use crate::nova_embeddings::data_url::parse_nova_data_url;

#[test]
fn replays_the_nova_data_url_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/nova_data_url");
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            match str::from_utf8(&bytes) {
                Ok(text) => {
                    let parsed = parse_nova_data_url(text);
                    assert_eq!(parsed.is_err(), name.ends_with("-rejected"), "{name}");
                }
                Err(_source) => assert!(name.ends_with("-malformed"), "{name} is not UTF-8"),
            }
            Ok::<(), io::Error>(())
        })
        .unwrap();
}
