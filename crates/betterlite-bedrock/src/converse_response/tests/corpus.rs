#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use crate::converse_response::decode_converse_reply;

#[test]
fn replays_the_bedrock_converse_response_corpus() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz")
        .join("corpus")
        .join("bedrock_converse_response");
    let mut seeds: Vec<PathBuf> = fs::read_dir(&directory)
        .expect("the corpus directory exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "the corpus holds its seeds");

    let mut accepted = 0;
    for path in seeds {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is text")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let outcome = decode_converse_reply(&bytes);
        assert_eq!(outcome.is_ok(), name.starts_with("valid-"), "seed {name}");
        if let Ok(reply) = &outcome {
            accepted = accepted.saturating_add(1);
            assert_eq!(
                reply.get("output").is_some(),
                name != "valid-empty-object.json",
                "seed {name}"
            );
        }
    }
    assert_eq!(accepted, 4, "every valid seed decodes");
}
