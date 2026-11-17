#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::{Action, Input};

#[test]
fn replays_the_responses_sse_recovery_seeds() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("responses_sse_recovery"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(!seeds.is_empty(), "no responses_sse_recovery seed exists");
    assert!(seeds.iter().all(|source| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        let input: Input = serde_json::from_value(case.call.input.clone())
            .expect("the seed input has the typed shape");
        assert_eq!(input.action, Action::RecoverRecordedOutput);
        assert!(!input.recordings.is_empty());
        case.call.operation == "responses_sse_recovery"
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
