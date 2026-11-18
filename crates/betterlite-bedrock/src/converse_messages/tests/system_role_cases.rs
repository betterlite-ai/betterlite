use betterlite_prompts::arguments::json::Json;

use crate::converse_messages::system::transform_system_message;
use crate::model_routes::ParamMap;

use super::support::{NOTE, blocks, expected, json_list, messages, render, text_turn};

fn transform(history: &str) -> (Json, Json) {
    let (kept, system) = transform_system_message(&messages(history), None, &ParamMap::new())
        .expect("the messages transform");
    (render(&kept), blocks(&system))
}

const QUESTION: &str = r#"{"role": "user", "content": "question"}"#;
const AGAIN: &str = r#"{"role": "user", "content": "again"}"#;
const REPLY: &str = r#"{"role": "assistant", "content": "reply"}"#;
const DEVELOPER: &str = r#"{"role": "developer", "content": "policy"}"#;
const SYSTEM_RULES: &str = r#"{"role": "system", "content": "rules"}"#;
const SYSTEM_PARTS_HOIST: &str = r#"{"role": "system", "content": [{"type": "text", "text": "a"}, {"type": "image", "source": {"bytes": "AA=="}}, {"type": "text", "text": ""}, {"type": "text", "text": " "}]}"#;
const SYSTEM_HOISTED: &str = r#"{"role": "system", "content": "hoisted"}"#;
const SYSTEM_LATER: &str = r#"{"role": "system", "content": "later"}"#;
const SYSTEM_HOLD: &str = r#"{"role": "system", "content": "hold"}"#;
const SYSTEM_SPACE: &str = r#"{"role": "system", "content": " "}"#;
const SYSTEM_EMPTY: &str = r#"{"role": "system", "content": ""}"#;
const SYSTEM_NULL: &str = r#"{"role": "system", "content": null}"#;
const SYSTEM_LIST_EMPTY: &str = r#"{"role": "system", "content": []}"#;
const SYSTEM_IMAGE: &str = r#"{"role": "system", "content": [{"type": "image"}]}"#;
const SYSTEM_IMAGE_SOURCED: &str =
    r#"{"role": "system", "content": [{"type": "image", "source": {"bytes": "AA=="}}]}"#;
const SYSTEM_TEXTLESS: &str = r#"{"role": "system", "content": [{"type": "text"}]}"#;
const SYSTEM_BLANK_TEXT: &str = r#"{"role": "system", "content": [{"type": "text", "text": null}, {"type": "text", "text": ""}]}"#;
const SYSTEM_PARTS_CACHE: &str = r#"{"role": "system", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "5m"}, "extra": 1}, {"type": "image"}, {"type": "text", "text": "b"}]}"#;
const SYSTEM_NAMED_CACHE: &str = r#"{"role": "system", "content": "a", "name": "bot", "cache_control": {"type": "ephemeral"}, "extra": 1}"#;
const RULES_BLOCKS: &str = r#"[{"text": "rules"}, {"text": "a"}, {"text": " "}]"#;

pub(super) fn system_role_as_user() {
    hoists_only_the_leading_run();
    a_developer_message_ends_the_run();
    systems_without_text_hoist_nothing();
    later_system_becomes_a_text_turn();
    parts_keep_their_cache_control();
    string_content_keeps_the_message_control();
    whitespace_content_becomes_a_text_turn();
    drops_later_systems_without_text();
}

fn hoists_only_the_leading_run() {
    let history = json_list(&[SYSTEM_RULES, SYSTEM_PARTS_HOIST, QUESTION]);
    let (kept, system) = transform(&history);
    assert_eq!(system, expected(RULES_BLOCKS));
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn a_developer_message_ends_the_run() {
    let history = json_list(&[SYSTEM_HOISTED, DEVELOPER, SYSTEM_LATER]);
    let (kept, system) = transform(&history);
    assert_eq!(system, expected(r#"[{"text": "hoisted"}]"#));
    let wanted = format!("[{DEVELOPER}, {}]", text_turn("later"));
    assert_eq!(kept, expected(&wanted));
}

fn systems_without_text_hoist_nothing() {
    let history = json_list(&[SYSTEM_EMPTY, SYSTEM_IMAGE, QUESTION]);
    let (kept, system) = transform(&history);
    assert_eq!(system, expected("[]"));
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn later_system_becomes_a_text_turn() {
    let history = json_list(&[QUESTION, SYSTEM_HOLD, AGAIN]);
    let (kept, _) = transform(&history);
    let wanted = format!("[{QUESTION}, {}, {AGAIN}]", text_turn("hold"));
    assert_eq!(kept, expected(&wanted));
}

fn parts_keep_their_cache_control() {
    let turn = cached_parts_turn();
    let history = json_list(&[QUESTION, SYSTEM_PARTS_CACHE, REPLY]);
    let (kept, _) = transform(&history);
    let wanted = expected(&json_list(&[QUESTION, turn.as_str(), REPLY]));
    assert_eq!(kept, wanted);
}

fn cached_parts_turn() -> String {
    format!(
        r#"{{"role": "user", "content": [{{"type": "text", "text": "{NOTE}"}}, {{"type": "text", "text": "a", "cache_control": {{"type": "ephemeral", "ttl": "5m"}}}}, {{"type": "text", "text": "b"}}]}}"#
    )
}

fn string_content_keeps_the_message_control() {
    let turn = named_cached_turn();
    let history = json_list(&[QUESTION, SYSTEM_NAMED_CACHE, REPLY]);
    let (kept, _) = transform(&history);
    let wanted = expected(&json_list(&[QUESTION, turn.as_str(), REPLY]));
    assert_eq!(kept, wanted);
}

fn named_cached_turn() -> String {
    format!(
        r#"{{"role": "user", "content": [{{"type": "text", "text": "{NOTE}"}}, {{"type": "text", "text": "a", "cache_control": {{"type": "ephemeral"}}}}]}}"#
    )
}

fn whitespace_content_becomes_a_text_turn() {
    let history = json_list(&[QUESTION, SYSTEM_SPACE, REPLY]);
    let (kept, _) = transform(&history);
    let wanted = format!("[{QUESTION}, {}, {REPLY}]", text_turn(" "));
    assert_eq!(kept, expected(&wanted));
}

fn drops_later_systems_without_text() {
    a_null_content_system_is_dropped();
    an_empty_string_system_is_dropped();
    an_empty_content_list_system_is_dropped();
    an_image_only_system_is_dropped();
    a_textless_part_system_is_dropped();
    a_blank_text_system_is_dropped();
}

fn an_empty_string_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_EMPTY, REPLY]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}, {REPLY}]")));
}

fn a_null_content_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_NULL]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn an_empty_content_list_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_LIST_EMPTY]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn an_image_only_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_IMAGE_SOURCED]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn a_textless_part_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_TEXTLESS]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}

fn a_blank_text_system_is_dropped() {
    let history = json_list(&[QUESTION, SYSTEM_BLANK_TEXT]);
    let (kept, _) = transform(&history);
    assert_eq!(kept, expected(&format!("[{QUESTION}]")));
}
