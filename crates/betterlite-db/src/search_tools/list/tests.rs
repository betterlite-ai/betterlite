#![cfg(test)]

use crate::auxiliary_repository::repository::TableError;
use crate::search_tools::registry::SearchToolRegistry;
use crate::search_tools::tests::fakes::{
    FakeReconnect, FakeTable, stored_record, transport_failure,
};

const REASON: &str = "get_all_search_tools_from_db_lookup_failure";

fn reconnecting(succeeds: bool) -> FakeReconnect {
    FakeReconnect {
        reasons: Vec::new(),
        succeeds,
    }
}

#[tokio::test]
async fn list_requests_created_at_in_descending_order_and_keeps_the_returned_order() {
    let table = FakeTable::with_rows(vec![
        stored_record("tool-2", "newer"),
        stored_record("tool-1", "older"),
    ]);
    let mut client = reconnecting(true);
    let tools = SearchToolRegistry::get_all_search_tools_from_db(&table, Some(&mut client))
        .await
        .unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool.search_tool_name.as_str())
        .collect();
    assert_eq!(names, ["newer", "older"]);
    assert_eq!(tools[0].search_tool_id.as_deref(), Some("tool-2"));
    assert_eq!(
        tools[0].updated_at.as_deref(),
        Some("2026-03-04T05:06:07.123456+00:00")
    );
    assert_eq!(table.calls(), ["find_many:Some(Descending)"]);
    assert!(client.reasons.is_empty());
}

#[tokio::test]
async fn list_retries_once_after_a_reconnect_following_a_transport_error() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    table
        .many
        .lock()
        .unwrap()
        .push_back(Err(transport_failure()));
    let mut client = reconnecting(true);
    let tools = SearchToolRegistry::get_all_search_tools_from_db(&table, Some(&mut client))
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(client.reasons, [REASON]);
    assert_eq!(
        table.calls(),
        ["find_many:Some(Descending)", "find_many:Some(Descending)"]
    );
}

#[tokio::test]
async fn list_does_not_reconnect_for_an_error_that_is_not_a_transport_error() {
    let table = FakeTable::default();
    table
        .many
        .lock()
        .unwrap()
        .push_back(Err(TableError::MissingRow));
    let mut client = reconnecting(true);
    let error = SearchToolRegistry::get_all_search_tools_from_db(&table, Some(&mut client))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting search tools from DB: statement returned no row"
    );
    assert!(client.reasons.is_empty());
    assert_eq!(table.calls().len(), 1);
}

#[tokio::test]
async fn list_reports_the_transport_error_when_the_reconnect_fails() {
    let table = FakeTable::default();
    table
        .many
        .lock()
        .unwrap()
        .push_back(Err(transport_failure()));
    let mut client = reconnecting(false);
    let error = SearchToolRegistry::get_all_search_tools_from_db(&table, Some(&mut client))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Error getting search tools from DB: No DB Connected.")
    );
    assert_eq!(table.calls().len(), 1);
}

#[tokio::test]
async fn list_without_a_reconnect_client_returns_the_first_error() {
    let table = FakeTable::default();
    table
        .many
        .lock()
        .unwrap()
        .push_back(Err(transport_failure()));
    let error = SearchToolRegistry::get_all_search_tools_from_db::<_, FakeReconnect>(&table, None)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Error getting search tools from DB: ")
    );
    assert_eq!(table.calls().len(), 1);
}

#[tokio::test]
async fn list_error_text_carries_the_source_chain() {
    let table = FakeTable::default();
    let source = serde_json::from_str::<u8>("x").unwrap_err();
    let expected =
        format!("Error getting search tools from DB: stored row is not valid JSON: {source}");
    table
        .many
        .lock()
        .unwrap()
        .push_back(Err(TableError::Row(source)));
    let error = SearchToolRegistry::get_all_search_tools_from_db::<_, FakeReconnect>(&table, None)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected);
}

#[tokio::test]
async fn list_names_the_field_of_an_unreadable_column() {
    let mut record = stored_record("tool-1", "web");
    record.search_tool_info = Some(crate::mcp_stored_contracts::JsonValue::Bool(true));
    let table = FakeTable::with_rows(vec![record]);
    let error = SearchToolRegistry::get_all_search_tools_from_db::<_, FakeReconnect>(&table, None)
        .await
        .unwrap_err();
    assert!(error.to_string().starts_with(
        "Error getting search tools from DB: search tool field search_tool_info is not valid"
    ));
}
