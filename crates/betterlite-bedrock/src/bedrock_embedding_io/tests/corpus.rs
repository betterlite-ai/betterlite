use std::fs;
use std::io;
use std::path::Path;

use crate::bedrock_embedding_io::decode::decode_embedding_response;

fn outcome(bytes: &[u8]) -> &'static str {
    if decode_embedding_response(bytes).is_ok() {
        "accepted"
    } else {
        "rejected"
    }
}

// each seed is named for the category a body of its shape must reach
#[test]
fn replays_the_bedrock_embedding_io_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_embedding_io");
    fs::read_dir(corpus)
        .expect("the embedding corpus is present")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("the seed name is UTF-8");
            let category = if name.starts_with("valid") {
                "accepted"
            } else {
                "rejected"
            };
            assert_eq!(outcome(&bytes), category, "seed {name}");
            Ok::<(), io::Error>(())
        })
        .expect("the embedding corpus replays");
}
