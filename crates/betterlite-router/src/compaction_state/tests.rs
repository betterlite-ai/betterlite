#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::router_compaction::ContextCompactionConfig;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{
    CompactionState, SharedCompactionState, compaction_pending, initialize_compaction_state,
};
use crate::compaction_history::contracts::Surface;
use crate::router_predicates::JsonMap;

fn armed() -> SharedCompactionState<()> {
    handle(Some(ContextCompactionConfig::default()))
}

fn unarmed() -> SharedCompactionState<()> {
    handle(None)
}

fn handle(config: Option<ContextCompactionConfig>) -> SharedCompactionState<()> {
    let state = CompactionState {
        config,
        candidates: Vec::new(),
        summary: None,
        parent_model: None,
        surface: Some(Surface::Responses),
    };
    Arc::new(Mutex::new(state))
}

fn payload(entries: Vec<(&str, Json)>) -> JsonMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(payload(entries))
}

fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

#[test]
fn pending_needs_a_state() {
    assert!(!compaction_pending::<()>(None, &JsonMap::new()));
}

#[test]
fn pending_needs_an_armed_state() {
    assert!(!compaction_pending(Some(&unarmed()), &JsonMap::new()));
}

#[test]
fn pending_is_true_for_an_armed_plain_request() {
    assert!(compaction_pending(Some(&armed()), &JsonMap::new()));
}

#[test]
fn pending_ignores_an_encrypted_block_in_messages() {
    let block = object(vec![("type", text("encrypted_content"))]);
    let message = object(vec![("role", text("user")), ("content", list(vec![block]))]);
    let request = payload(vec![("messages", list(vec![message]))]);
    assert!(compaction_pending(Some(&armed()), &request));
}

#[rstest]
#[case::previous_response_id("previous_response_id")]
#[case::conversation("conversation")]
#[case::context_management("context_management")]
#[case::compaction("compaction")]
fn pending_rejects_each_client_managed_key(#[case] key: &str) {
    let request = payload(vec![(key, text("client-side"))]);
    assert!(!compaction_pending(Some(&armed()), &request));
}

#[rstest]
#[case::previous_response_id("previous_response_id")]
#[case::conversation("conversation")]
#[case::context_management("context_management")]
#[case::compaction("compaction")]
fn pending_ignores_each_client_managed_key_set_to_null(#[case] key: &str) {
    let request = payload(vec![(key, Json::Null)]);
    assert!(compaction_pending(Some(&armed()), &request));
}

#[test]
fn pending_lets_a_plain_input_item_through() {
    let block = object(vec![("type", text("input_text")), ("text", text("hi"))]);
    let item = object(vec![
        ("type", text("message")),
        ("content", list(vec![block])),
    ]);
    let request = payload(vec![("input", list(vec![item]))]);
    assert!(compaction_pending(Some(&armed()), &request));
}

#[rstest]
#[case::reasoning("reasoning")]
#[case::compaction("compaction")]
#[case::item_reference("item_reference")]
fn pending_rejects_each_client_managed_item_kind(#[case] kind: &str) {
    let item = object(vec![("type", text(kind))]);
    let request = payload(vec![("input", list(vec![item]))]);
    assert!(!compaction_pending(Some(&armed()), &request));
}

#[test]
fn pending_rejects_an_item_with_encrypted_content() {
    let item = object(vec![
        ("type", text("message")),
        ("encrypted_content", text("opaque")),
    ]);
    let request = payload(vec![("input", list(vec![item]))]);
    assert!(!compaction_pending(Some(&armed()), &request));
}

#[test]
fn pending_ignores_null_encrypted_content() {
    let item = object(vec![
        ("type", text("message")),
        ("encrypted_content", Json::Null),
    ]);
    let request = payload(vec![("input", list(vec![item]))]);
    assert!(compaction_pending(Some(&armed()), &request));
}

#[test]
fn pending_rejects_an_item_with_an_encrypted_block() {
    let block = object(vec![("type", text("encrypted_content"))]);
    let item = object(vec![
        ("type", text("message")),
        ("content", list(vec![block])),
    ]);
    let request = payload(vec![("input", list(vec![item]))]);
    assert!(!compaction_pending(Some(&armed()), &request));
}

#[test]
fn pending_ignores_a_string_input() {
    let request = payload(vec![("input", text("one more question"))]);
    assert!(compaction_pending(Some(&armed()), &request));
}

#[test]
fn initialize_reuses_the_existing_handle() {
    let existing = armed();
    let reused = initialize_compaction_state(Some(&existing), Surface::Chat);
    assert!(Arc::ptr_eq(&reused, &existing));
}

#[test]
fn initialize_keeps_the_existing_surface() {
    let existing = armed();
    let reused = initialize_compaction_state(Some(&existing), Surface::Chat);
    let kept = reused.lock().unwrap();
    assert_eq!(kept.surface, Some(Surface::Responses));
    assert!(kept.config.is_some());
    drop(kept);
}

#[test]
fn initialize_builds_a_fresh_state_with_only_the_surface() {
    let created = initialize_compaction_state::<()>(None, Surface::Chat);
    let fresh = created.lock().unwrap();
    assert_eq!(fresh.surface, Some(Surface::Chat));
    assert!(fresh.config.is_none());
    assert!(fresh.candidates.is_empty());
    assert!(fresh.summary.is_none());
    assert!(fresh.parent_model.is_none());
    drop(fresh);
}

#[test]
fn initialize_shares_a_write_between_clones() {
    let first = initialize_compaction_state::<()>(None, Surface::Chat);
    let second = initialize_compaction_state(Some(&first), Surface::Chat);
    let mut written = second.lock().unwrap();
    written.candidates.push("compactor".to_owned());
    written.parent_model = Some("parent".to_owned());
    drop(written);
    let read = first.lock().unwrap();
    assert_eq!(read.candidates, ["compactor"]);
    assert_eq!(read.parent_model.as_deref(), Some("parent"));
    drop(read);
}
