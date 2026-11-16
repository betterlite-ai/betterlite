#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

fn run_case_file(file: &str) -> Value {
    let path = Path::new(ORACLE_DIR)
        .join("cases/helpers/vertex-agent-chunk")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    super::Op::decode_and_run(&case.call.input).unwrap()
}

#[test]
fn first_dictionary_text_keeps_the_first_text_part() {
    let output = run_case_file("first-dictionary-text.yaml");
    assert_eq!(
        output,
        json!({
            "chunks": [
                {"content": "first", "role": "assistant", "finish_reason": null, "usage": null},
                {"content": "after scalars", "role": "assistant", "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null}
            ]
        })
    );
}

#[test]
fn empty_text_keeps_its_content_and_names_no_role() {
    let output = run_case_file("empty-text-role.yaml");
    assert_eq!(
        output,
        json!({
            "chunks": [
                {"content": "", "role": null, "finish_reason": null, "usage": null},
                {"content": "", "role": null, "finish_reason": null, "usage": null},
                {"content": "", "role": null, "finish_reason": null, "usage": null}
            ]
        })
    );
}

#[test]
fn the_finish_reason_is_lowercased_before_it_is_mapped() {
    let output = run_case_file("lowercase-finish.yaml");
    assert_eq!(
        output,
        json!({
            "chunks": [
                {"content": null, "role": null, "finish_reason": "stop", "usage": null},
                {"content": null, "role": null, "finish_reason": "length", "usage": null},
                {"content": null, "role": null, "finish_reason": "tool_calls", "usage": null},
                {"content": null, "role": null, "finish_reason": "stop", "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": "stop", "usage": null},
                {"content": null, "role": null, "finish_reason": "content_filter", "usage": null},
                {"content": "done", "role": "assistant", "finish_reason": "stop", "usage": null}
            ]
        })
    );
}

#[test]
fn a_partial_usage_metadata_defaults_the_missing_counts() {
    let output = run_case_file("partial-usage.yaml");
    assert_eq!(
        output,
        json!({
            "chunks": [
                {"content": null, "role": null, "finish_reason": null, "usage": [7_u64, 0_u64, 0_u64]},
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": null, "usage": [1_u64, 2_u64, 3_u64]},
                {"content": "x", "role": "assistant", "finish_reason": null, "usage": [0_u64, 0_u64, 9_u64]},
                {"content": null, "role": null, "finish_reason": null, "usage": [0_u64, 0_u64, 0_u64]}
            ]
        })
    );
}

#[test]
fn a_state_delta_is_ignored() {
    let output = run_case_file("ignored-state-delta.yaml");
    assert_eq!(
        output,
        json!({
            "chunks": [
                {"content": null, "role": null, "finish_reason": null, "usage": null},
                {"content": "streamed", "role": "assistant", "finish_reason": null, "usage": null},
                {"content": null, "role": null, "finish_reason": "stop", "usage": null},
                {"content": "real", "role": "assistant", "finish_reason": null, "usage": null}
            ]
        })
    );
}

#[test]
fn helper_seeds_select_vertex_agent_chunk() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_agent_chunk"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "vertex_agent_chunk"
            && super::Op::decode_and_run(&case.call.input).is_ok()
    }));
}
