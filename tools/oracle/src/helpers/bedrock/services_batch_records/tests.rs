#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

use super::Op;

const OPERATION: &str = "bedrock_services_batch_records";

#[test]
fn helper_seeds_select_bedrock_services_batch_records() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(OPERATION))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            (path.display().to_string(), source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no {OPERATION} seed exists");
    let cases: Vec<HelperCase> = seeds
        .iter()
        .map(|(path, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            validate_helper(Path::new(path), &case).unwrap();
            assert_eq!(case.call.operation, OPERATION);
            case
        })
        .collect();
    let rejected: Vec<String> = cases
        .iter()
        .filter_map(|case| Op::check(&case.call.input).err())
        .collect();
    assert!(rejected.is_empty(), "{rejected:#?}");
}

#[test]
fn replays_the_bedrock_services_batch_records_cases() {
    let cases =
        Path::new(ORACLE_DIR).join("cases/helpers/providers/bedrock-services-batch-records");
    let mut replayed = 0_usize;
    for entry in fs::read_dir(cases).unwrap() {
        let source = fs::read_to_string(entry.unwrap().path()).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        assert!(
            matches!(
                run_case(&[crate::GROUP], &case, false).unwrap(),
                HelperRun::Match
            ),
            "{} does not match LiteLLM",
            case.name
        );
        replayed = replayed.saturating_add(1);
    }
    assert!(
        replayed >= 10,
        "only {replayed} batch record cases replayed"
    );
}
