#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::invoke_response::tests::counter::FixtureCounter;
use crate::invoke_response::tests::support::{decode_with, fixture, headers};

#[test]
fn replays_the_bedrock_invoke_response_corpus() {
    let seeds = replay_bedrock_invoke_response_corpus();
    assert!(seeds >= 10, "the corpus holds only {seeds} seeds");
}

fn replay_bedrock_invoke_response_corpus() -> usize {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_invoke_response");
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory exists") {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let mut case = fixture();
        case.headers = headers("4", "2");
        let counter = FixtureCounter::counting(1, 1);
        let run = decode_with(&case, &bytes, &counter);
        if name.starts_with("valid-") {
            assert!(run.reply.is_ok(), "{name}: {}", run.message());
        } else {
            assert!(run.reply.is_err(), "{name} parsed as a reply");
        }
        seeds = seeds.saturating_add(1);
    }
    seeds
}
