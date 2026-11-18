#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::input::Input;

#[test]
fn helper_seeds_select_bedrock_other_embedding_jobs() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_other_embedding_jobs"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert!(
        !seeds.is_empty(),
        "no bedrock_other_embedding_jobs seed exists"
    );
    let functions: BTreeSet<&str> = seeds
        .iter()
        .map(|source| {
            let case: HelperCase =
                serde_saphyr::from_str(source).expect("a seed parses as a helper case");
            let input: Input = serde_json::from_value(case.call.input.clone())
                .expect("the seed input has the typed shape");
            assert!(
                case.call.operation == "bedrock_other_embedding_jobs"
                    && validate_helper(Path::new("<helper>"), &case).is_ok(),
                "the seed must call the bedrock_other_embedding_jobs operation"
            );
            function_name(&input)
        })
        .collect();
    let expected: BTreeSet<&str> = [
        "async_invoke_status_url",
        "status_body_passthrough",
        "status_http_failure",
        "status_logging",
    ]
    .into_iter()
    .collect();
    assert_eq!(functions, expected, "every operation needs a seed");
}

const fn function_name(input: &Input) -> &'static str {
    match input {
        Input::AsyncInvokeStatusUrl(_) => "async_invoke_status_url",
        Input::StatusBodyPassthrough(_) => "status_body_passthrough",
        Input::StatusHttpFailure(_) => "status_http_failure",
        Input::StatusLogging(_) => "status_logging",
    }
}
