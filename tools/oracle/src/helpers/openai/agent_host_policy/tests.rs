#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

fn seed_cases() -> Vec<HelperCase> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("agent_host_policy"))
        })
        .map(|path| {
            let source = fs::read_to_string(path).expect("seed file");
            serde_saphyr::from_str(&source).expect("helper seed")
        })
        .collect()
}

#[test]
fn helper_seeds_select_agent_host_policy() {
    let cases = seed_cases();
    assert!(!cases.is_empty());
    assert!(cases.iter().all(|case: &HelperCase| {
        validate_helper(Path::new("<helper>"), case).is_ok()
            && case.call.operation == "agent_host_policy"
    }));
}

#[test]
fn helper_seeds_cover_every_host_and_inception_fim() {
    let combined = seed_cases()
        .iter()
        .map(|case| case.call.input.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        ["gradient", "heroku", "hyperbolic", "lambda_ai", "inception"]
            .iter()
            .all(|host| combined.contains(&format!("\"host\":\"{host}\"")))
    );
    assert!(combined.contains("\"op\":\"fim_supported\""));
    assert!(combined.contains("\"op\":\"fim_params\""));
}
