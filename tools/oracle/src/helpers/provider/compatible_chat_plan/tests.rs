#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::invoke;

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("the helper corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("compatible_chat_plan"))
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
fn helper_seeds_select_compatible_chat_plan() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let rejected: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let decodes = serde_json::from_value::<super::Input>(case.call.input.clone()).is_ok();
            case.call.operation != "compatible_chat_plan"
                || !decodes
                || validate_helper(Path::new("<helper>"), case).is_err()
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(rejected.is_empty(), "{rejected:?}");
}

#[test]
fn helper_seeds_answer_every_call() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let failed: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| {
            let calls = case.call.input["calls"].as_array().map_or(0, Vec::len);
            let output = invoke(&case.call);
            let answered = output
                .value
                .as_ref()
                .and_then(|value| value["calls"].as_array())
                .is_some_and(|answers| answers.len() == calls);
            !output.ok || !answered
        })
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(failed.is_empty(), "{failed:?}");
}
