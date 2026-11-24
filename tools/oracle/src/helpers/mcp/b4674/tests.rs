#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::{HelperCase, check_helper_calls, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
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

#[test]
fn every_oracle_case_decodes_into_the_helper_input() {
    let cases = cases("cases/helpers/mcp-gateway-dcr");
    assert_eq!(cases.len(), 15);
    cases.into_iter().for_each(|(path, case)| {
        validate_helper(&path, &case).unwrap();
        assert_eq!(case.call.operation, "b4674");
        check_helper_calls(&path, &case, &[crate::GROUP]).unwrap();
    });
}

#[test]
fn every_helper_seed_decodes_into_the_helper_input() {
    let seeds = seed_cases();
    assert!(!seeds.is_empty());
    seeds.into_iter().for_each(|(path, case)| {
        validate_helper(&path, &case).unwrap();
        assert_eq!(case.call.operation, "b4674");
        check_helper_calls(&path, &case, &[crate::GROUP]).unwrap();
    });
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

#[test]
fn a_moved_or_damaged_seal_does_not_open() {
    let scenarios = vec![
        scenario("tamper", Wrap::Tamper),
        scenario("truncate", Wrap::Truncate),
        scenario("swap", Wrap::SwapPrefix),
        OpenScenario {
            name: "raw".to_owned(),
            payload: None,
            prefix: SealPrefix::Client,
            wrap: Wrap::None,
            raw: Some("llm_dcrc_hello".to_owned()),
        },
    ];
    let report = open::reports(&scenarios, &cipher()).unwrap();
    report.iter().for_each(|entry| {
        assert_eq!(entry["opened"], json!(false));
        assert_eq!(entry["redirect_uris"], json!(null));
    });
}
