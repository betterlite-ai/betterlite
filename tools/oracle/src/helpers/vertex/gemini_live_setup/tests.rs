#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

use super::Op;

const OPERATION: &str = "gemini_live_setup";

#[test]
fn replays_the_gemini_live_setup_helper_cases() {
    let folder = Path::new(ORACLE_DIR).join("cases/helpers/gemini-live-setup");
    let mut paths: Vec<_> = fs::read_dir(folder)
        .expect("the case folder exists")
        .map(|entry| entry.expect("a case entry is readable").path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("yaml"))
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the folder holds helper cases");
    for path in paths {
        let source = fs::read_to_string(&path).expect("the case is readable text");
        let case: HelperCase = serde_saphyr::from_str(&source).expect("the case parses");
        validate_helper(&path, &case).expect("the case is valid");
        assert_eq!(case.call.operation, OPERATION);
        let outcome = run_case(&[crate::GROUP], &case, false).expect("the case runs");
        assert!(
            matches!(outcome, HelperRun::Match),
            "{} does not match LiteLLM",
            path.display()
        );
    }
}

#[test]
fn replays_the_gemini_live_setup_seeds() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .expect("the helper corpus exists")
        .map(|entry| entry.expect("a corpus entry is readable").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(OPERATION))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("the seed is readable text");
            let parsed: HelperCase = serde_saphyr::from_str(&source).expect("the seed parses");
            validate_helper(Path::new("<helper>"), &parsed)
                .expect("the seed is a valid helper case");
            parsed
        })
        .collect();
    assert!(!seeds.is_empty(), "the corpus holds a {OPERATION} seed");
    for seed in &seeds {
        assert_eq!(seed.call.operation, OPERATION);
        Op::check(&seed.call.input).expect("the seed input decodes into the typed input");
        for call in &seed.calls {
            assert_eq!(call.operation, OPERATION);
            Op::check(&call.input).expect("the extra seed input decodes into the typed input");
        }
    }
}
