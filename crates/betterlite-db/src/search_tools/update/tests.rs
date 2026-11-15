#![cfg(test)]

use crate::auxiliary_repository::repository::TableError;
use crate::search_tools::tests::fakes::{
    EXPECTED_PARAMS, FakeTable, INFO_TEXT, instant, registry, stored_record, tool,
};

#[tokio::test]
async fn update_sends_serialized_fields_with_a_fresh_updated_at() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let mut renamed = tool();
    renamed.search_tool_name = "renamed".to_owned();
    registry()
        .update_search_tool_in_db("tool-1", &renamed, &table)
        .await
        .unwrap();
    let sent = table.updated.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].search_tool_name, "renamed");
    assert_eq!(sent[0].litellm_params, EXPECTED_PARAMS);
    assert_eq!(sent[0].search_tool_info, INFO_TEXT);
    assert_eq!(sent[0].updated_at, instant());
    assert_eq!(table.calls(), ["update:tool-1"]);
}

#[tokio::test]
async fn update_returns_the_row_with_iso_timestamps_and_the_original_created_at() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let row = registry()
        .update_search_tool_in_db("tool-1", &tool(), &table)
        .await
        .unwrap();
    assert_eq!(row.search_tool_id, "tool-1");
    assert_eq!(row.search_tool_name, "web");
    assert_eq!(row.created_at, "2026-02-25T06:13:20+00:00");
    assert_eq!(row.updated_at, "2026-03-04T05:06:07.123456+00:00");
}

#[tokio::test]
async fn update_replaces_absent_info_with_an_empty_object() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let mut bare = tool();
    bare.search_tool_info = None;
    registry()
        .update_search_tool_in_db("tool-1", &bare, &table)
        .await
        .unwrap();
    assert_eq!(table.updated.lock().unwrap()[0].search_tool_info, "{}");
}

#[tokio::test]
async fn update_of_a_missing_tool_carries_the_litellm_message_prefix() {
    let table = FakeTable::default();
    let error = registry()
        .update_search_tool_in_db("missing", &tool(), &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error updating search tool in DB: statement returned no row"
    );
}

#[tokio::test]
async fn update_failure_is_wrapped() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    table.fail("update", TableError::MissingRow);
    let error = registry()
        .update_search_tool_in_db("tool-1", &tool(), &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error updating search tool in DB: statement returned no row"
    );
}
