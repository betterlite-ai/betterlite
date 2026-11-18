#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::parse_helper_case;
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::types::Input;

const OPERATION: &str = "bedrock_invoke_stream";
// Seeds are named <operation>-<case>.yaml, so the dash keeps a longer operation's seeds out.
const SEED_PREFIX: &str = "bedrock_invoke_stream-";

#[test]
fn helper_seeds_select_bedrock_invoke_stream() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the corpus folder is readable")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(SEED_PREFIX))
        })
        .collect();
    assert!(!seeds.is_empty(), "no {SEED_PREFIX} seed exists");
    assert!(seeds.iter().all(|path| {
        let source = fs::read_to_string(path).expect("a seed is readable");
        let case = parse_helper_case(path, &source).expect("a seed parses");
        assert_eq!(case.call.operation, OPERATION);
        serde_json::from_value::<Input>(case.call.input)
            .is_ok_and(|input| matches!(input, Input::TransportArguments { .. }))
    }));
}
