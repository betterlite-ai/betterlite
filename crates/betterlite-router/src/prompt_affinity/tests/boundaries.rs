use std::sync::Arc;

use betterlite_core::message_utilities::prompt_cache_checks::PromptCacheSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::support::{
    FakePins, MARKED_MESSAGES, ToolBlocks, cache_key, catalog, deployment, ids,
    image_message_source, json_messages, lookback, messages, pin, position_numbers, positions,
    request, service, success, text_blocks_source, token_counter, tool_blocks_source,
};
use crate::prompt_affinity::service::{CountingGate, TokenGate as _};

const THREE_BLOCKS: &str = r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"},{"type":"text","text":"c","cache_control":{"type":"ephemeral"}}]}]"#;

pub(super) async fn run(name: &str) {
    match name {
        "a_prompt_at_the_minimum_reads_the_pin" => a_prompt_at_the_minimum_reads_the_pin().await,
        "a_counting_gate_compares_the_prompt_against_the_minimum" => {
            a_counting_gate_compares_the_prompt_against_the_minimum().await;
        }
        "an_unknown_model_uses_the_default_minimum" => {
            an_unknown_model_uses_the_default_minimum().await;
        }
        "a_deployment_without_a_model_name_uses_the_default" => {
            a_deployment_without_a_model_name_uses_the_default().await;
        }
        "the_lookback_window_holds_twenty_positions" => {
            the_lookback_window_holds_twenty_positions().await;
        }
        "a_tool_result_run_counts_as_one_position" => a_tool_result_run_counts_as_one_position(),
        "a_tool_result_run_in_a_user_message_counts_once" => {
            a_tool_result_run_in_a_user_message_counts_once();
        }
        "consecutive_tool_messages_count_once" => consecutive_tool_messages_count_once(),
        "an_empty_content_list_hashes_like_missing_content" => {
            an_empty_content_list_hashes_like_missing_content();
        }
        "an_empty_content_string_hashes_like_missing_content" => {
            an_empty_content_string_hashes_like_missing_content();
        }
        "a_tool_use_run_followed_by_a_tool_result_run_counts_two_positions" => {
            a_tool_use_run_followed_by_a_tool_result_run_counts_two_positions();
        }
        "a_value_that_is_not_a_pin_is_skipped" => a_value_that_is_not_a_pin_is_skipped().await,
        "an_empty_prompt_has_no_prefix" => an_empty_prompt_has_no_prefix().await,
        "a_prompt_without_a_mark_never_pins" => a_prompt_without_a_mark_never_pins().await,
        "long_base64_payloads_hash_by_size_only" => long_base64_payloads_hash_by_size_only(),
        _ => panic!("unknown case {name}"),
    }
}

async fn a_prompt_at_the_minimum_reads_the_pin() {
    let pins = Arc::new(FakePins::default());
    let raw = json_messages(MARKED_MESSAGES);
    pins.seed(&cache_key(&raw, None).unwrap(), pin("dep-a"));
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-plain"),
    ];
    let typed = messages(MARKED_MESSAGES);
    let at = service(Arc::clone(&pins), 400, false);
    let selected = at
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a"]);
    let below = service(Arc::clone(&pins), 399, false);
    let untouched = below
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&untouched), ["dep-a", "dep-b"]);
}

async fn a_counting_gate_compares_the_prompt_against_the_minimum() {
    let gate = CountingGate {
        counter: Arc::new(token_counter()),
        settings: PromptCacheSettings::default(),
        catalog: catalog(),
    };
    let raw = json_messages(MARKED_MESSAGES);
    assert!(gate.meets_minimum("gpt-4o", &raw, Some(1)).await);
    assert!(!gate.meets_minimum("gpt-4o", &raw, Some(u64::MAX)).await);
}

async fn an_unknown_model_uses_the_default_minimum() {
    the_default_minimum_applies(["unknown-model", "unknown-model"]).await;
}

async fn a_deployment_without_a_model_name_uses_the_default() {
    the_default_minimum_applies(["", ""]).await;
}

async fn the_default_minimum_applies(models: [&str; 2]) {
    let pins = Arc::new(FakePins::default());
    let raw = json_messages(MARKED_MESSAGES);
    pins.seed(&cache_key(&raw, None).unwrap(), pin("dep-a"));
    let pool = [
        deployment("dep-a", models[0]),
        deployment("dep-b", models[1]),
    ];
    let typed = messages(MARKED_MESSAGES);
    let at = service(Arc::clone(&pins), 1024, false);
    let selected = at
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a"]);
    let below = service(Arc::clone(&pins), 1023, false);
    let untouched = below
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&untouched), ["dep-a", "dep-b"]);
}

async fn the_lookback_window_holds_twenty_positions() {
    let raw = json_messages(&text_blocks_source(25));
    let found = positions(&raw);
    assert_eq!(found.len(), 25);
    assert_eq!(lookback(&raw).len(), 20);
    let inside = &found[5];
    let outside = &found[4];
    let pins = Arc::new(FakePins::default());
    pins.seed(&inside.cache_key, pin("dep-a"));
    let seen = service(Arc::clone(&pins), 2000, false);
    assert_eq!(
        seen.pin_for(Some(&raw), None).await.unwrap().as_deref(),
        Some("dep-a")
    );
    let older = Arc::new(FakePins::default());
    older.seed(&outside.cache_key, pin("dep-a"));
    let missed = service(Arc::clone(&older), 2000, false);
    assert_eq!(missed.pin_for(Some(&raw), None).await.unwrap(), None);
}

fn a_tool_result_run_counts_as_one_position() {
    let run = json_messages(&tool_blocks_source(21, ToolBlocks::Result));
    let used = json_messages(&tool_blocks_source(21, ToolBlocks::Use));
    let blocks = json_messages(&text_blocks_source(21));
    assert_eq!(lookback(&run).len(), 21);
    assert_eq!(lookback(&used).len(), 21);
    assert_eq!(lookback(&blocks).len(), 20);
}

// a tool_result block carries the run type even when its message has no tool role
fn a_tool_result_run_in_a_user_message_counts_once() {
    let raw = json_messages(
        r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"one"},{"type":"tool_result","tool_use_id":"call-2","content":"two"},{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(position_numbers(&raw), [1, 1, 2]);
}

// the tool role alone makes the tool_result run type, with no tool_result block to read it from
fn consecutive_tool_messages_count_once() {
    let raw = json_messages(
        r#"[{"role":"tool","content":"42"},{"role":"tool","content":"43"},{"role":"user","content":[{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(position_numbers(&raw), [1, 1, 2]);
}

fn an_empty_content_list_hashes_like_missing_content() {
    let empty = json_messages(
        r#"[{"role":"user","content":[]},{"role":"user","content":[{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let missing = json_messages(
        r#"[{"role":"user"},{"role":"user","content":[{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(positions(&empty), positions(&missing));
    assert_eq!(position_numbers(&empty), [1, 2]);
}

fn an_empty_content_string_hashes_like_missing_content() {
    let empty = json_messages(
        r#"[{"role":"user","content":""},{"role":"user","content":[{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let missing = json_messages(
        r#"[{"role":"user"},{"role":"user","content":[{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(positions(&empty), positions(&missing));
    assert_eq!(position_numbers(&empty), [1, 2]);
}

// a tool_use run ends where the tool_result run begins, so the two never merge into one position
fn a_tool_use_run_followed_by_a_tool_result_run_counts_two_positions() {
    let raw = json_messages(
        r#"[{"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"lookup","input":{}},{"type":"tool_use","id":"call-2","name":"lookup","input":{}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"one"},{"type":"tool_result","tool_use_id":"call-2","content":"two"},{"type":"text","text":"last","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(position_numbers(&raw), [1, 1, 2, 2, 3]);
}

async fn a_value_that_is_not_a_pin_is_skipped() {
    let raw = json_messages(THREE_BLOCKS);
    let found = positions(&raw);
    assert_eq!(found.len(), 3);
    let pins = Arc::new(FakePins::default());
    pins.seed(&found[2].cache_key, Json::Bool(true));
    let mut wrong = IndexMap::new();
    wrong.insert("model_id".to_owned(), Json::Number(Number::from(7_u8)));
    pins.seed(&found[1].cache_key, Json::Object(wrong));
    let affinity = service(Arc::clone(&pins), 2000, false);
    assert_eq!(affinity.pin_for(Some(&raw), None).await.unwrap(), None);
    pins.seed(&found.first().unwrap().cache_key, pin("dep-b"));
    assert_eq!(
        affinity.pin_for(Some(&raw), None).await.unwrap().as_deref(),
        Some("dep-b")
    );
}

async fn an_empty_prompt_has_no_prefix() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let empty: Vec<Json> = Vec::new();
    assert_eq!(cache_key(&empty, None), None);
    assert_eq!(affinity.pin_for(Some(&empty), None).await.unwrap(), None);
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages("[]");
    let selected = affinity
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a", "dep-b"]);
}

async fn a_prompt_without_a_mark_never_pins() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let raw = json_messages(r#"[{"role":"user","content":"hi"}]"#);
    affinity
        .record_success(&success("completion", Some("dep-a"), Some(&raw)))
        .await
        .unwrap();
    assert!(pins.keys().is_empty());
    assert_eq!(cache_key(&raw, None), None);
}

fn long_base64_payloads_hash_by_size_only() {
    let first = json_messages(&image_message_source(&"A".repeat(100)));
    let second = json_messages(&image_message_source(&"B".repeat(100)));
    let longer = json_messages(&image_message_source(&"C".repeat(120)));
    assert_eq!(cache_key(&first, None), cache_key(&second, None));
    assert_ne!(cache_key(&first, None), cache_key(&longer, None));
    let short_one = json_messages(&image_message_source(&"D".repeat(64)));
    let short_two = json_messages(&image_message_source(&"E".repeat(64)));
    assert_ne!(cache_key(&short_one, None), cache_key(&short_two, None));
}
