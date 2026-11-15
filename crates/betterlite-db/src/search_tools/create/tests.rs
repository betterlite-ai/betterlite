#![cfg(test)]

use crate::auxiliary_repository::repository::TableError;
use crate::search_tools::tests::fakes::{
    EXPECTED_PARAMS, FakeTable, INFO_TEXT, instant, registry, tool,
};

#[tokio::test]
async fn create_persists_json_text_with_credentials_and_one_timestamp() {
    let table = FakeTable::default();
    registry()
        .add_search_tool_to_db(tool(), &table)
        .await
        .unwrap();
    let stored = table.created.lock().unwrap().clone();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].search_tool_name, "web");
    assert_eq!(stored[0].litellm_params, EXPECTED_PARAMS);
    assert_eq!(stored[0].search_tool_info, INFO_TEXT);
    assert_eq!(stored[0].created_at, instant());
    assert_eq!(stored[0].updated_at, instant());
    assert_eq!(table.calls(), ["create"]);
}

#[tokio::test]
async fn create_returns_the_input_with_the_id_and_iso_timestamps_of_the_stored_row() {
    let table = FakeTable::default();
    let created = registry()
        .add_search_tool_to_db(tool(), &table)
        .await
        .unwrap();
    assert_eq!(created.search_tool_id.as_deref(), Some("generated-id"));
    assert_eq!(created.search_tool_name, "web");
    assert_eq!(created.litellm_params.search_provider, "tavily");
    assert_eq!(created.litellm_params.max_retries, Some(3));
    assert_eq!(
        created.created_at.as_deref(),
        Some("2026-03-04T05:06:07.124456+00:00")
    );
    assert_eq!(
        created.updated_at.as_deref(),
        Some("2026-03-04T05:06:07.124456+00:00")
    );
    assert_eq!(created.search_tool_info.unwrap().len(), 1);
}

#[tokio::test]
async fn create_stores_an_empty_object_when_the_info_is_absent() {
    let table = FakeTable::default();
    let mut bare = tool();
    bare.search_tool_info = None;
    let created = registry()
        .add_search_tool_to_db(bare, &table)
        .await
        .unwrap();
    assert_eq!(table.created.lock().unwrap()[0].search_tool_info, "{}");
    assert!(created.search_tool_info.is_none());
}

#[tokio::test]
async fn create_failure_carries_the_litellm_message_prefix() {
    let table = FakeTable::default();
    table.fail("create", TableError::MissingRow);
    let error = registry()
        .add_search_tool_to_db(tool(), &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error adding search tool to DB: statement returned no row"
    );
}
