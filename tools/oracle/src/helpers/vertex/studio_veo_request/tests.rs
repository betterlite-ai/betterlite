#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[test]
fn helper_seeds_select_studio_veo_request() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_veo_request"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_veo_request"
            && super::Op::check(&case.call.input).is_ok()
    }));
}

#[rstest]
fn matches_the_pinned_litellm_studio_veo_request(
    #[base_dir = "../../.."]
    #[files("cases/helpers/studio-veo-request/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("the case runs"),
        HelperRun::Match
    ));
}

#[test]
fn an_unknown_request_field_is_rejected() {
    let input = serde_json::json!({
        "requests": [{"model": "m", "prompt": "p", "bogus": 1_i32}],
    });
    assert!(super::Op::check(&input).is_err());
}
