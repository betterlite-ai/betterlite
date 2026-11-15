#![cfg(test)]

use crate::auxiliary_repository::repository::TableError;
use crate::search_tools::delete::DeletedSearchTool;
use crate::search_tools::tests::fakes::{FakeTable, registry, stored_record};

#[tokio::test]
async fn delete_success_returns_the_message_and_the_deleted_name() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let deleted = registry()
        .delete_search_tool_from_db("tool-1", &table)
        .await
        .unwrap();
    assert_eq!(
        deleted,
        DeletedSearchTool {
            message: "Search tool tool-1 deleted successfully".to_owned(),
            search_tool_name: "web".to_owned(),
        }
    );
    assert_eq!(table.calls(), ["find_unique:tool-1", "delete:tool-1"]);
    assert!(table.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn delete_not_found_raises_the_exact_litellm_text_without_deleting() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    let error = registry()
        .delete_search_tool_from_db("missing", &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error deleting search tool from DB: Search tool with ID missing not found"
    );
    assert_eq!(table.calls(), ["find_unique:missing"]);
    assert_eq!(table.rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn delete_lookup_failure_skips_the_delete() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    table.fail("find_unique", TableError::MissingRow);
    let error = registry()
        .delete_search_tool_from_db("tool-1", &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error deleting search tool from DB: statement returned no row"
    );
    assert_eq!(table.calls(), ["find_unique:tool-1"]);
}

#[tokio::test]
async fn delete_failure_after_the_lookup_is_wrapped() {
    let table = FakeTable::with_rows(vec![stored_record("tool-1", "web")]);
    table.fail("delete", TableError::MissingRow);
    let error = registry()
        .delete_search_tool_from_db("tool-1", &table)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error deleting search tool from DB: statement returned no row"
    );
    assert_eq!(table.calls(), ["find_unique:tool-1", "delete:tool-1"]);
}
