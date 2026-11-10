#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
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
    for (path, case) in &seeds {
        assert_eq!(case.call.operation, "bytez_record", "{path}");
        assert!(
            validate_helper(Path::new("<helper>"), case).is_ok(),
            "{path}"
        );
    }
}

#[test]
fn helper_seeds_normalize_messages_and_count_usage() {
    let mut normalized = 0_usize;
    let mut counted = 0_usize;
    for (path, case) in seeds() {
        let output = invoke(&case.call);
        let Some(value) = output.value else {
            continue;
        };
        if let Some(prompt_tokens) = value
            .get("value")
            .and_then(|value| value.get("prompt_tokens"))
            .and_then(serde_json::Value::as_i64)
        {
            assert!(prompt_tokens > 0, "{path}");
            counted += 1;
        }
        let Some(messages) = value
            .get("value")
            .and_then(|value| value.get("normalized"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        assert!(!messages.is_empty(), "{path}");
        assert!(
            messages.iter().any(|message| message["content"]
                .as_array()
                .is_some_and(|blocks| !blocks.is_empty())),
            "{path}"
        );
        normalized += 1;
    }
    assert!(normalized > 0);
    assert!(counted > 0);
}
