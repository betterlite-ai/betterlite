use std::fs;
use std::path::Path;

use crate::pegasus::reply::decode_pegasus_reply;

use super::support::{FixtureCounter, block_on, context, single_choice};

#[test]
fn replays_the_bedrock_pegasus_corpus() {
    let seeds = replay_bedrock_pegasus_corpus();
    assert!(seeds >= 10, "the corpus holds only {seeds} seeds");
}

fn replay_bedrock_pegasus_corpus() -> usize {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_pegasus");
    let headers = vec![
        (
            "x-amzn-bedrock-input-token-count".to_owned(),
            "4".to_owned(),
        ),
        (
            "x-amzn-bedrock-output-token-count".to_owned(),
            "2".to_owned(),
        ),
    ];
    let mut seeds = 0_usize;
    for entry in fs::read_dir(corpus).expect("the corpus directory exists") {
        let path = entry.expect("the corpus entry reads").path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("the seed name is UTF-8")
            .to_owned();
        let bytes = fs::read(&path).expect("the seed reads");
        let mut response = single_choice();
        let outcome = block_on(decode_pegasus_reply(
            &bytes,
            &context(200, &headers, &[], 0.0),
            &mut response,
            &FixtureCounter::counting(0, 0),
        ));
        if name.starts_with("valid-") {
            assert!(outcome.is_ok(), "{name}: {outcome:?}");
        } else {
            assert!(outcome.is_err(), "{name} parsed and assigned content");
        }
        seeds = seeds.saturating_add(1);
    }
    seeds
}
