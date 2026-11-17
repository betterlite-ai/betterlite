#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use serde_json::Value;

const CASE_NAMES: [&str; 8] = [
    "both-references",
    "caller-file-preserved",
    "create-failure-cleans-file",
    "fetch-failure-cleans-both",
    "invalid-duration-after-log",
    "missing-input",
    "uploaded-success",
    "url-without-upload",
];

#[derive(serde::Deserialize)]
struct SeedCall {
    call: BTreeMap<String, Value>,
}

fn seed_calls(source: &str) -> bool {
    let seed: SeedCall = serde_saphyr::from_str(source).expect("the seed call parses");
    let Some(value) = seed.call.get("soniox_pipeline") else {
        return false;
    };
    let input: super::input::Input =
        serde_json::from_value(value.clone()).expect("the seed input has the typed shape");
    !input.pipeline.answers.is_empty()
}

fn case_sources() -> Vec<(String, String)> {
    let folder =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../cases/helpers/soniox-pipeline");
    let mut sources: Vec<(String, String)> = fs::read_dir(folder)
        .expect("the soniox-pipeline cases should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|suffix| suffix == "yaml"))
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("a case has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a case is readable");
            (name, source)
        })
        .collect();
    sources.sort();
    sources
}

#[test]
fn types_every_soniox_pipeline_oracle_case() {
    let cases = case_sources();
    let names: Vec<&str> = cases.iter().map(|(name, _source)| name.as_str()).collect();
    assert_eq!(names, CASE_NAMES.as_slice());
    assert!(cases.iter().all(|(_name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a case parses as a helper case");
        validate_helper(Path::new("<case>"), &case).is_ok()
            && case.call.operation == "soniox_pipeline"
            && serde_json::from_value::<super::input::Input>(case.call.input).is_ok()
    }));
}

#[test]
fn replays_the_soniox_pipeline_seeds() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("soniox_pipeline"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no soniox_pipeline seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        assert_eq!(case.call.operation, "soniox_pipeline", "{name}");
        validate_helper(Path::new("<helper>"), &case).is_ok() && seed_calls(source)
    }));
}
