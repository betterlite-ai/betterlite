#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::json;

use super::Op;

#[rstest]
fn matches_the_pinned_end_user_id_lookup(
    #[base_dir = "../../.."]
    #[files("cases/helpers/end-user-id/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}

#[test]
fn helper_seeds_select_the_end_user_id_lookup() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("end-user-id-"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "b10050"
            && Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn the_standard_customer_id_wins_over_the_body() {
    let answer = Op::decode_and_run(&json!({
        "user_header_mappings": [],
        "user_header_name": null,
        "validate_end_user_id_in_db": false,
        "requests": [
            {
                "body": {},
                "headers": [
                    ["x-litellm-end-user-id", "end-user"],
                    ["x-litellm-customer-id", "customer"],
                ],
            },
            {"body": {"user": "body"}, "headers": null},
            {"body": {"user": 1.5}, "headers": null},
        ],
    }))
    .expect("the lookup answers");
    assert_eq!(answer, json!({"ids": ["customer", "body", "1.5"]}));
}
