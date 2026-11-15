#![cfg(test)]

use crate::auxiliary_repository::repository::TableError;
use crate::mcp_stored_contracts::JsonValue;
use crate::search_tools::tests::fakes::{FakeTable, registry, stored_record};

#[tokio::test]
async fn get_by_id_returns_the_typed_tool_with_iso_timestamps() {
    let table = FakeTable::with_rows(vec![
        stored_record("tool-1", "web"),
        stored_record("tool-2", "other"),
    ]);
    let found = registry()
        .get_search_tool_by_id_from_db("tool-2", &table)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.search_tool_id.as_deref(), Some("tool-2"));
    assert_eq!(found.search_tool_name, "other");
    assert_eq!(found.litellm_params.search_provider, "tavily");
    assert_eq!(found.search_tool_info.unwrap().len(), 1);
    assert_eq!(
        found.created_at.as_deref(),
        Some("2026-02-25T06:13:20+00:00")
    );
    assert_eq!(
        found.updated_at.as_deref(),
        Some("2026-03-04T05:06:07.123456+00:00")
    );
    assert_eq!(table.calls(), ["find_unique:tool-2"]);
}

#[tokio::test]
async fn get_by_id_returns_none_for_a_missing_tool() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let found = registry()
        .get_search_tool_by_id_from_db("missing", &table)
        .await
        .unwrap();
    assert!(found.is_none());
    assert_eq!(table.calls(), ["find_unique:missing"]);
}

#[tokio::test]
async fn get_by_id_reads_columns_stored_as_json_objects() {
    let mut record = stored_record("tool-1", "web");
    record.litellm_params =
        serde_json::from_str::<JsonValue>(r#"{"search_provider":"exa"}"#).unwrap();
    record.search_tool_info = None;
    let table = FakeTable::with_rows(vec![record]);
    let found = registry()
        .get_search_tool_by_id_from_db("tool-1", &table)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.litellm_params.search_provider, "exa");
    assert!(found.search_tool_info.is_none());
}

#[tokio::test]
async fn get_by_id_names_the_field_of_an_unreadable_column() {
    let mut record = stored_record("tool-1", "web");
    record.litellm_params = JsonValue::String("{".to_owned());
    let table = FakeTable::with_rows(vec![record]);
    let error = registry()
        .get_search_tool_by_id_from_db("tool-1", &table)
        .await
        .unwrap_err();
    assert!(error.to_string().starts_with(
        "Error getting search tool from DB: search tool field litellm_params is not valid"
    ));
}

#[tokio::test]
async fn get_by_id_failure_is_wrapped() {
    let table = FakeTable::default();
    table.fail("find_unique", TableError::MissingRow);
    let error = registry()
        .get_search_tool_by_id_from_db("tool-1", &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting search tool from DB: statement returned no row"
    );
}
