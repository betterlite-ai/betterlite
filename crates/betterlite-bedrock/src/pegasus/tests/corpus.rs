use std::fs;
use std::path::{Path, PathBuf};

use crate::pegasus::reply::decode_pegasus_reply;

use super::support::{FixtureCounter, block_on, context, single_choice};

#[test]
fn replays_the_bedrock_pegasus_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_pegasus");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no bedrock_pegasus seeds");
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
    for path in &paths {
        let name = path.file_name().unwrap().to_str().unwrap();
        let bytes = fs::read(path).unwrap();
        let mut response = single_choice();
        let outcome = block_on(decode_pegasus_reply(
            &bytes,
            &context(&headers, &[], 0.0),
            &mut response,
            &FixtureCounter::counting(0, 0),
        ));
        if name.starts_with("valid-") {
            assert!(outcome.is_ok(), "{name}: {outcome:?}");
        } else {
            assert!(outcome.is_err(), "{name} parsed and assigned content");
        }
    }
}
