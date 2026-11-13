#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::compare::helper::HelperOutput;
use crate::helpers::invoke;

fn seeds() -> Vec<(String, HelperCase)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .expect("corpus")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bytez_record"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("seed");
            let case: HelperCase = serde_saphyr::from_str(&source).expect("parsed seed");
            (path.display().to_string(), case)
        })
        .collect();
    seeds.sort_by(|left, right| left.0.cmp(&right.0));
    seeds
}

#[test]
fn helper_seeds_select_bytez_record() {
    let seeds = seeds();
    assert!(!seeds.is_empty());
    let wrong: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| case.call.operation != "bytez_record")
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
    let invalid: Vec<&str> = seeds
        .iter()
        .filter(|(_, case)| validate_helper(Path::new("<helper>"), case).is_err())
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(invalid.is_empty(), "{invalid:?}");
}

fn prompt_tokens(output: &HelperOutput) -> Option<i64> {
    output
        .value
        .as_ref()
        .and_then(|value| value.pointer("/value/prompt_tokens"))
        .and_then(Value::as_i64)
}

fn has_blocks(output: &HelperOutput) -> bool {
    output
        .value
        .as_ref()
        .and_then(|value| value.pointer("/value/normalized"))
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message["content"]
                    .as_array()
                    .is_some_and(|blocks| !blocks.is_empty())
            })
        })
}

#[test]
fn helper_seeds_normalize_messages_and_count_usage() {
    let outputs: Vec<(String, HelperOutput)> = seeds()
        .into_iter()
        .map(|(path, case)| (path, invoke(&case.call)))
        .collect();
    assert!(
        outputs
            .iter()
            .any(|(_, output)| prompt_tokens(output).is_some())
    );
    assert!(outputs.iter().any(|(_, output)| has_blocks(output)));
    let uncounted: Vec<&str> = outputs
        .iter()
        .filter(|(_, output)| prompt_tokens(output).is_some_and(|tokens| tokens <= 0))
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(uncounted.is_empty(), "{uncounted:?}");
    let empty: Vec<&str> = outputs
        .iter()
        .filter(|(_, output)| value_has_normalized(output) && !has_blocks(output))
        .map(|(path, _)| path.as_str())
        .collect();
    assert!(empty.is_empty(), "{empty:?}");
}

fn value_has_normalized(output: &HelperOutput) -> bool {
    output
        .value
        .as_ref()
        .and_then(|value| value.pointer("/value/normalized"))
        .is_some()
}
