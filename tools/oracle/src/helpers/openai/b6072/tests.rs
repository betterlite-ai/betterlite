#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[test]
fn helper_seeds_select_b6072() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let sources: Vec<String> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fireworks_ai_chat-"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed"))
        .collect();
    assert!(!sources.is_empty());
    let rejected: Vec<&str> = sources
        .iter()
        .filter(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).expect("a helper case");
            let decodes =
                serde_json::from_value::<super::input::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "b6072"
                || !decodes
                || validate_helper(Path::new("<helper>"), &case).is_err()
        })
        .map(String::as_str)
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[rstest]
fn matches_pinned_b6072(
    #[base_dir = "../../.."]
    #[files("cases/helpers/fireworks-ai-chat/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
