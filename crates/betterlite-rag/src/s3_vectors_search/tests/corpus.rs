use std::fs;
use std::io;
use std::path::Path;

use betterlite_provider::api::vector_store_contracts::config::Headers;

use crate::s3_vectors_search::response::decode_s3_vectors_search;

#[test]
fn replays_the_s3_vectors_search_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/s3_vectors_search");
    let mut seen = 0_u32;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            let outcome = decode_s3_vectors_search(200, &Headers::new(), &bytes, None);
            let expected_ok = name.starts_with("valid-") || name.starts_with("boundary-valid-");
            assert_eq!(outcome.is_ok(), expected_ok, "{name}");
            seen = seen.saturating_add(1);
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(seen >= 12, "expected committed seeds, found {seen}");
}
