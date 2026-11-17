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
    let names: Vec<String> = seeds
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("the seed name is text")
                .to_owned()
        })
        .collect();
    let outcomes: Vec<_> = seeds
        .iter()
        .map(|path| decode_converse_reply(&fs::read(path).expect("the seed reads")))
        .collect();
    names.iter().zip(&outcomes).for_each(|(name, outcome)| {
        assert_eq!(outcome.is_ok(), name.starts_with("valid-"), "seed {name}");
        if let Ok(reply) = outcome {
            assert_eq!(
                reply.body.contains_key("output"),
                name != "valid-empty-object.json",
                "seed {name}"
            );
        }
    });
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        4,
        "every valid seed decodes"
    );
}
