use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support;
use crate::api::responses_ws_history::history::get_history_messages;
use crate::api::responses_ws_history::{ManagedResponsesWebSocket, SESSION_HISTORY_BUDGET_BYTES};
use crate::api::responses_ws_managed::process::process_response_create;

const FIRST_FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const DROPPED_FRAME: &str =
    r#"{"type": "response.create", "input": "again", "previous_response_id": "r1"}"#;
const UNKNOWN_FRAME: &str =
    r#"{"type": "response.create", "input": "again", "previous_response_id": "resp_nope"}"#;
const FIRST: &str = r#"{"type": "response.completed", "response": {"id": "r1", "output": [{"type": "message", "content": [{"type": "output_text", "text": "A"}]}]}}"#;
const SECOND: &str = r#"{"type": "response.completed", "response": {"id": "r2", "output": []}}"#;

fn message(text: &str) -> IndexMap<String, Json> {
    support::object(&format!(r#"{{"text":"{text}"}}"#))
}

fn messages(text: &str) -> Vec<IndexMap<String, Json>> {
    vec![message(text)]
}

fn capped(
    shared: &support::Shared,
    budget: usize,
) -> ManagedResponsesWebSocket<support::ScriptedSink> {
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(shared)));
    handler.session_history_budget_bytes = budget;
    handler
}

fn keys(handler: &ManagedResponsesWebSocket<support::ScriptedSink>) -> Vec<&str> {
    handler.session_history.keys().map(String::as_str).collect()
}

#[test]
fn a_new_connection_has_the_sixteen_mib_history_budget() {
    let shared = support::shared();
    let handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));

    assert_eq!(SESSION_HISTORY_BUDGET_BYTES, 16 * 1024 * 1024);
    assert_eq!(
        handler.session_history_budget_bytes,
        SESSION_HISTORY_BUDGET_BYTES
    );
    assert_eq!(handler.session_history_bytes, 0);
    assert!(handler.session_history.is_empty());
}

#[tokio::test]
async fn a_list_at_the_budget_is_stored_and_one_byte_over_warns_once() {
    let shared = support::shared();
    let mut handler = capped(&shared, 14);
    handler.store_history("r1".to_owned(), messages("a"));

    let ((), logs) = support::capture(async {
        handler.store_history("r2".to_owned(), messages("aa"));
    })
    .await;

    assert_eq!(keys(&handler), ["r1"]);
    assert_eq!(handler.session_history_bytes, 14);
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: not storing 15 bytes of history for response_id=r2; the budget is 14 bytes"
        ]
    );
}

#[test]
fn storing_past_the_budget_drops_the_oldest_entries_until_the_new_one_fits() {
    let shared = support::shared();
    let mut handler = capped(&shared, 40);
    handler.store_history("r1".to_owned(), messages("aaaaaaa"));
    handler.store_history("r2".to_owned(), messages("aaaaaaa"));
    assert_eq!(handler.session_history_bytes, 40);

    handler.store_history("r3".to_owned(), messages("aaaaaaaaaaaa"));

    assert_eq!(keys(&handler), ["r3"]);
    assert_eq!(handler.session_history_bytes, 25);
    assert!(get_history_messages(&handler, "r1").is_empty());
    assert!(get_history_messages(&handler, "r2").is_empty());
    assert_eq!(
        get_history_messages(&handler, "r3"),
        messages("aaaaaaaaaaaa")
    );
}

#[test]
fn the_newest_turn_is_kept_when_it_alone_fills_the_budget() {
    let shared = support::shared();
    let mut handler = capped(&shared, 14);
    handler.store_history("r1".to_owned(), messages("a"));

    handler.store_history("r2".to_owned(), messages("a"));

    assert_eq!(keys(&handler), ["r2"]);
    assert_eq!(handler.session_history_bytes, 14);
}

#[test]
fn storing_an_id_again_replaces_it_and_counts_only_the_new_list() {
    let shared = support::shared();
    let mut handler = capped(&shared, 40);
    handler.store_history("r1".to_owned(), messages("aaaaaaa"));
    handler.store_history("r2".to_owned(), messages("aaaaaaa"));

    handler.store_history("r1".to_owned(), messages("aaaaaaa"));
    assert_eq!(keys(&handler), ["r2", "r1"]);
    assert_eq!(handler.session_history_bytes, 40);

    handler.store_history("r1".to_owned(), messages("aaaaaaaaaaaa"));
    assert_eq!(keys(&handler), ["r1"]);
    assert_eq!(handler.session_history_bytes, 25);
}

#[tokio::test]
async fn a_turn_over_the_budget_is_not_stored_and_warns_once() {
    let shared = support::shared();
    let mut handler = capped(&shared, 16);
    let provider = support::ScriptedProvider::new(
        &shared,
        vec![
            support::script(vec![support::event(FIRST)]),
            support::script(vec![support::event(SECOND)]),
        ],
    );

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        FIRST_FRAME,
    ))
    .await;
    outcome.unwrap();
    assert!(handler.session_history.is_empty());
    assert_eq!(handler.session_history_bytes, 0);
    assert_eq!(logs.len(), 1);
    assert!(logs[0].starts_with("ManagedResponsesWS: not storing "));
    assert!(logs[0].ends_with("; the budget is 16 bytes"));

    let (later, later_logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        DROPPED_FRAME,
    ))
    .await;
    later.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(
        recorded[1].kwargs.get("input"),
        Some(&support::json(r#""again""#))
    );
    assert_eq!(
        recorded[1].kwargs.get("previous_response_id"),
        Some(&support::json(r#""r1""#))
    );
    assert_eq!(
        later_logs[0],
        "ManagedResponsesWS: no in-memory history for previous_response_id=r1; falling back to DB-based session reconstruction"
    );
}

#[tokio::test]
async fn a_turn_naming_a_dropped_or_unknown_id_gets_no_history() {
    let shared = support::shared();
    let mut handler = capped(&shared, 28);
    handler.store_history("r1".to_owned(), messages("a"));
    handler.store_history("r2".to_owned(), messages("a"));
    handler.store_history("r3".to_owned(), messages("a"));
    let provider = support::ScriptedProvider::new(
        &shared,
        vec![
            support::script(vec![support::event(SECOND)]),
            support::script(vec![support::event(SECOND)]),
        ],
    );

    let (dropped, dropped_logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        DROPPED_FRAME,
    ))
    .await;
    dropped.unwrap();
    let (unknown, unknown_logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        UNKNOWN_FRAME,
    ))
    .await;
    unknown.unwrap();

    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].kwargs.get("input"),
        Some(&support::json(r#""again""#))
    );
    assert_eq!(
        recorded[1].kwargs.get("input"),
        recorded[0].kwargs.get("input")
    );
    assert_eq!(
        recorded[0].kwargs.get("previous_response_id"),
        Some(&support::json(r#""r1""#))
    );
    assert_eq!(
        recorded[1].kwargs.get("previous_response_id"),
        Some(&support::json(r#""resp_nope""#))
    );
    assert_eq!(
        dropped_logs[0],
        "ManagedResponsesWS: no in-memory history for previous_response_id=r1; falling back to DB-based session reconstruction"
    );
    assert_eq!(
        unknown_logs[0],
        "ManagedResponsesWS: no in-memory history for previous_response_id=resp_nope; falling back to DB-based session reconstruction"
    );
    assert!(get_history_messages(&handler, "r1").is_empty());
}
