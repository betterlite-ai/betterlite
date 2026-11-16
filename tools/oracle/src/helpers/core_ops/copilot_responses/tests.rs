#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Input, Op};

#[derive(Deserialize)]
struct Call {
    copilot_responses: Input,
}

#[derive(Deserialize)]
struct Seed {
    call: Call,
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case")
}

fn answer(name: &str) -> Value {
    let source = fs::read_to_string(corpus().join(format!("copilot_responses_{name}")))
        .expect("seed should be readable");
    let seed: Seed = serde_saphyr::from_str(&source).expect("seed should parse");
    Op::run(&seed.call.copilot_responses).expect("the helper should answer")
}

#[test]
fn every_copilot_responses_seed_is_a_valid_helper_case() {
    let sources: Vec<String> = fs::read_dir(corpus())
        .expect("the helper corpus should exist")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("copilot_responses_"))
        })
        .map(|path| fs::read_to_string(path).expect("a seed is readable"))
        .collect();
    assert_eq!(sources.len(), 9_usize);
    assert!(sources.iter().all(|source| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        serde_saphyr::from_str::<Seed>(source).expect("a seed calls copilot_responses");
        validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}

#[test]
fn model_mode_decides_before_the_endpoint_list() {
    assert_eq!(
        answer("model_mode_opt_out"),
        json!([false, true, {"native_websocket": false}])
    );
}

#[test]
fn the_catalog_endpoint_list_decides_when_the_mode_is_silent() {
    assert_eq!(
        answer("catalog_endpoint"),
        json!([true, false, true, false])
    );
}

#[test]
fn only_a_typed_input_attribute_sets_the_initiator() {
    let answers = answer("typed_input_only");
    let initiators: Vec<Option<&str>> = answers
        .as_array()
        .expect("a list of steps")
        .iter()
        .map(|step| step["headers"]["X-Initiator"].as_str())
        .collect();
    assert_eq!(
        initiators,
        [Some("agent"), Some("user"), None, None, Some("agent")]
    );
}

#[test]
fn a_tool_role_alone_stays_a_user_turn() {
    let answers = answer("tool_role_user");
    assert_eq!(answers[0]["headers"]["X-Initiator"], "user");
    assert_eq!(answers[1]["headers"]["X-Initiator"], "agent");
    assert_eq!(answers[2]["raised"], 401_i32);
}

#[test]
fn the_vision_walk_stops_beyond_the_loaded_depth() {
    let answers = answer("depth_boundary");
    let found: Vec<bool> = answers
        .as_array()
        .expect("a list of steps")
        .iter()
        .map(|step| step["headers"]["copilot-vision-request"] == "true")
        .collect();
    assert_eq!(found, [true, false, true, false]);
}

#[test]
fn the_responses_url_follows_the_base_precedence() {
    assert_eq!(
        answer("base_precedence"),
        json!([
            "https://explicit.test/responses",
            "https://cached.test/responses",
            "https://env.test/responses",
            "https://api.githubcopilot.com/responses"
        ])
    );
}

#[test]
fn stream_ids_follow_the_anchor_and_a_delta_before_it_passes() {
    let changing = answer("changing_item_ids");
    assert_eq!(changing[0][1]["item_id"], "rs_1");
    assert_eq!(changing[0][4]["nested_id"], "rs_1");
    assert_eq!(changing[1][1]["item_id"], "rs_1");
    assert_eq!(changing[1][1]["nested_id"], "nested");
    let early = answer("pre_anchor_delta");
    assert_eq!(early[0][0]["item_id"], "early");
    assert_eq!(early[0][2]["item_id"], "rs_1");
}

#[test]
fn reasoning_items_lose_their_null_fields() {
    let kept = answer("null_reasoning_fields");
    assert_eq!(
        kept[0],
        json!([{"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "enc"}])
    );
    assert_eq!(
        kept[1],
        json!([{"type": "reasoning", "id": "rs_2", "summary": []}])
    );
}
