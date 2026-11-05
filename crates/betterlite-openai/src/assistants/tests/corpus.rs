#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::assistants::decode::decode_assistant_object;
use crate::assistants::error::AssistantsError;

// The file extension names the outcome a seed must produce.
#[test]
fn replays_the_openai_assistants_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_assistants");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let outcome = match decode_assistant_object(&bytes) {
                Ok(_assistant) => "ok",
                Err(AssistantsError::Truncated { .. }) => "truncated",
                Err(AssistantsError::Malformed { .. }) => "malformed",
                Err(AssistantsError::Shape { .. }) => "shape",
                Err(other) => panic!("{name} failed outside decoding: {other}"),
            };
            assert_eq!(
                path.extension().unwrap().to_str().unwrap(),
                outcome,
                "{name}"
            );
            replayed += 1;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed >= 8, "{replayed} seeds replayed");
}
