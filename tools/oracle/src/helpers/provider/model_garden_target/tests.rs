#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("model_garden_target"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("a helper case");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn model_garden_target_seeds_select_the_operation() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<super::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "model_garden_target"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn model_garden_target_seeds_answer_every_call() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let failed: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| !matches!(run_case(&[crate::GROUP], case, false), Ok(HelperRun::Match)))
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
}
