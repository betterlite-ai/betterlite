#![cfg(test)]

use crate::gemini_live_client::state::{MAX_TOOL_CALL_NAMES, ToolCallNames};

fn names(map: &ToolCallNames) -> Vec<&str> {
    map.entries.keys().map(String::as_str).collect()
}

#[test]
fn a_call_without_an_id_or_a_name_is_not_recorded() {
    let mut map = ToolCallNames::default();
    map.insert("", "get_weather");
    map.insert("call_1", "");
    assert!(map.entries.is_empty());
}

#[test]
fn a_new_call_id_is_appended_at_the_recent_end() {
    let mut map = ToolCallNames::default();
    map.insert("call_1", "get_weather");
    map.insert("call_2", "get_time");
    assert_eq!(names(&map), ["call_1", "call_2"]);
}

#[test]
fn reinserting_a_call_id_moves_it_to_the_recent_end() {
    let mut map = ToolCallNames::default();
    map.insert("call_1", "get_weather");
    map.insert("call_2", "get_time");
    map.insert("call_1", "get_weather");
    assert_eq!(names(&map), ["call_2", "call_1"]);
}

#[test]
fn refreshing_a_known_call_id_moves_it_to_the_recent_end() {
    let mut map = ToolCallNames::default();
    map.insert("call_1", "get_weather");
    map.insert("call_2", "get_time");
    map.refresh("call_1");
    assert_eq!(names(&map), ["call_2", "call_1"]);
}

#[test]
fn refreshing_an_unknown_call_id_changes_nothing() {
    let mut map = ToolCallNames::default();
    map.insert("call_1", "get_weather");
    map.refresh("call_9");
    assert_eq!(names(&map), ["call_1"]);
}

fn filled() -> ToolCallNames {
    let mut map = ToolCallNames::default();
    for index in 0..MAX_TOOL_CALL_NAMES {
        map.insert(&format!("call_{index}"), "name");
    }
    map
}

#[test]
fn the_oldest_call_id_is_evicted_past_the_cap() {
    let mut map = filled();
    map.insert("call_new", "name");
    assert_eq!(map.entries.len(), MAX_TOOL_CALL_NAMES);
    assert!(!map.entries.contains_key("call_0"));
    assert!(map.entries.contains_key("call_new"));
    assert_eq!(
        map.entries.keys().next().map(String::as_str),
        Some("call_1")
    );
}

#[test]
fn a_reinserted_call_id_does_not_evict_a_healthy_entry() {
    let mut map = filled();
    map.insert("call_0", "renamed");
    assert_eq!(map.entries.len(), MAX_TOOL_CALL_NAMES);
    assert_eq!(
        map.entries.get("call_0").map(String::as_str),
        Some("renamed")
    );
    assert_eq!(
        map.entries.keys().next().map(String::as_str),
        Some("call_1")
    );
}

#[test]
fn the_cap_keeps_the_newest_call_ids() {
    let map = filled();
    assert_eq!(map.entries.len(), MAX_TOOL_CALL_NAMES);
    let newest = MAX_TOOL_CALL_NAMES.saturating_sub(1);
    assert!(map.entries.contains_key(&format!("call_{newest}")));
}
