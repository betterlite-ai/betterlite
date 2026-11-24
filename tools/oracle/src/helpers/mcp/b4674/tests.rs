#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::{HelperCase, check_helper_calls, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use rstest::rstest;
use serde_json::json;

use super::{OpenScenario, SealPrefix, Wrap, cipher, open};

fn cases(folder: &str) -> Vec<(PathBuf, HelperCase)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(Path::new(ORACLE_DIR).join(folder))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            (path, case)
        })
        .collect()
}

fn seed_cases() -> Vec<(PathBuf, HelperCase)> {
    let mut seeds = cases("fuzz/corpus/helper_case");
    seeds.retain(|(path, _case)| seed_name(path));
    seeds
}

fn seed_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("mcp-gateway-dcr-"))
}

fn check_case(path: &Path, case: &HelperCase) -> Result<(), String> {
    validate_helper(path, case).map_err(|error| error.to_string())?;
    if case.call.operation != "b4674" {
        return Err(format!(
            "{} names {} instead of b4674",
            path.display(),
            case.call.operation
        ));
    }
    check_helper_calls(path, case, &[crate::GROUP]).map_err(|error| error.to_string())
}

#[rstest]
fn every_oracle_case_decodes_into_the_helper_input(
    #[base_dir = "../../.."]
    #[files("cases/helpers/mcp-gateway-dcr/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    check_case(&path, &case).unwrap();
}

#[test]
fn every_helper_seed_decodes_into_the_helper_input() {
    let seeds = seed_cases();
    assert!(!seeds.is_empty(), "the b4674 seed corpus is present");
    let problems: Vec<String> = seeds
        .iter()
        .filter_map(|(path, case)| check_case(path, case).err())
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
}

fn scenario(name: &str, wrap: Wrap) -> OpenScenario {
    OpenScenario {
        name: name.to_owned(),
        payload: Some(json!({
            "redirect_uris": ["https://client.example/callback"],
            "iat": 1_700_000_000_i64,
        })),
        prefix: SealPrefix::Client,
        wrap,
        raw: None,
    }
}

#[test]
fn a_sealed_client_opens_with_the_sent_uris() {
    let report = open::reports(&[scenario("roundtrip", Wrap::None)], &cipher()).unwrap();
    assert_eq!(report[0]["opened"], json!(true));
    assert_eq!(
        report[0]["redirect_uris"],
        json!(["https://client.example/callback"])
    );
    assert_eq!(report[0]["iat"], json!(1_700_000_000_i64));
}

#[rstest]
#[case::tamper(scenario("tamper", Wrap::Tamper))]
#[case::truncate(scenario("truncate", Wrap::Truncate))]
#[case::swap(scenario("swap", Wrap::SwapPrefix))]
#[case::raw(OpenScenario {
    name: "raw".to_owned(),
    payload: None,
    prefix: SealPrefix::Client,
    wrap: Wrap::None,
    raw: Some("llm_dcrc_hello".to_owned()),
})]
fn a_moved_or_damaged_seal_does_not_open(#[case] damaged: OpenScenario) {
    let report = open::reports(&[damaged], &cipher()).unwrap();
    assert_eq!(report[0]["opened"], json!(false));
    assert_eq!(report[0]["redirect_uris"], json!(null));
}
