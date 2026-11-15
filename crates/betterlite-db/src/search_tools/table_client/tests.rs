#![cfg(test)]

use chrono::{DateTime, Utc};

use crate::auxiliary_repository::repository::TableError;
use crate::mcp_stored_contracts::JsonValue;
use crate::postgres_store::errors::is_unique_violation;
use crate::search_tools::record::SearchToolRecord;

use super::postgres::PostgresSearchTools;
use super::{CreatedAtOrder, SearchToolCreate, SearchToolTableClient as _, SearchToolUpdate};

mod support;
use support::{Case, Journal, failing_publisher, recording_publisher, row_count};

const PARAMS: &str = r#"{"search_provider":"tavily"}"#;
const PUBLISHED: &str = "publish:litellm_searchtoolstable";
const NO_DB_MESSAGE: &str =
    "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";

fn at(micros: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(micros).unwrap()
}

fn create_data(name: &str, created_at: DateTime<Utc>) -> SearchToolCreate {
    SearchToolCreate {
        search_tool_name: name.to_owned(),
        litellm_params: PARAMS.to_owned(),
        search_tool_info: "{}".to_owned(),
        created_at,
        updated_at: created_at,
    }
}

fn update_data(name: &str, updated_at: DateTime<Utc>) -> SearchToolUpdate {
    SearchToolUpdate {
        search_tool_name: name.to_owned(),
        litellm_params: r#"{"search_provider":"exa"}"#.to_owned(),
        search_tool_info: r#"{"description":"changed"}"#.to_owned(),
        updated_at,
    }
}

fn published(journal: &Journal) -> Vec<String> {
    journal.lock().unwrap().clone()
}

#[tokio::test]
async fn table_client_create_stores_json_strings_and_publishes_the_change() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    let record = table
        .create(create_data("web", at(1_772_600_767_123_456)))
        .await
        .unwrap();
    assert_eq!(record.search_tool_id.len(), 36);
    assert_eq!(record.search_tool_name, "web");
    assert_eq!(record.litellm_params, JsonValue::String(PARAMS.to_owned()));
    assert_eq!(
        record.search_tool_info,
        Some(JsonValue::String("{}".to_owned()))
    );
    assert_eq!(record.created_at, at(1_772_600_767_123_000));
    assert_eq!(record.updated_at, at(1_772_600_767_123_000));
    assert_eq!(published(&journal), [PUBLISHED]);
    case.close().await;
}

#[tokio::test]
async fn table_client_find_unique_returns_the_row_or_none() {
    let case = Case::new().await;
    let table = PostgresSearchTools::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let created = table
        .create(create_data("web", at(1_772_600_767_000_000)))
        .await
        .unwrap();
    let found = table.find_unique(&created.search_tool_id).await.unwrap();
    assert_eq!(found, Some(created));
    assert_eq!(table.find_unique("missing").await.unwrap(), None);
    case.close().await;
}

#[tokio::test]
async fn table_client_find_many_orders_by_created_at() {
    let case = Case::new().await;
    let table = PostgresSearchTools::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    table
        .create(create_data("third", at(1_772_600_769_000_000)))
        .await
        .unwrap();
    table
        .create(create_data("first", at(1_772_600_767_000_000)))
        .await
        .unwrap();
    table
        .create(create_data("second", at(1_772_600_768_000_000)))
        .await
        .unwrap();
    let names = |rows: Vec<SearchToolRecord>| -> Vec<String> {
        rows.into_iter().map(|row| row.search_tool_name).collect()
    };
    let descending = table
        .find_many(Some(CreatedAtOrder::Descending))
        .await
        .unwrap();
    let ascending = table
        .find_many(Some(CreatedAtOrder::Ascending))
        .await
        .unwrap();
    assert_eq!(names(descending), ["third", "second", "first"]);
    assert_eq!(names(ascending), ["first", "second", "third"]);
    assert_eq!(table.find_many(None).await.unwrap().len(), 3);
    case.close().await;
}

#[tokio::test]
async fn table_client_update_replaces_fields_keeps_created_at_and_publishes() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    let created = table
        .create(create_data("web", at(1_772_600_767_000_000)))
        .await
        .unwrap();
    let updated = table
        .update(
            &created.search_tool_id,
            update_data("renamed", at(1_772_700_000_000_000)),
        )
        .await
        .unwrap();
    assert_eq!(updated.search_tool_id, created.search_tool_id);
    assert_eq!(updated.search_tool_name, "renamed");
    assert_eq!(
        updated.litellm_params,
        JsonValue::String(r#"{"search_provider":"exa"}"#.to_owned())
    );
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.updated_at, at(1_772_700_000_000_000));
    assert_eq!(published(&journal), [PUBLISHED, PUBLISHED]);
    case.close().await;
}

#[tokio::test]
async fn table_client_update_of_a_missing_row_fails_without_publishing() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    let error = table
        .update("missing", update_data("renamed", at(1_772_700_000_000_000)))
        .await
        .unwrap_err();
    assert!(matches!(error, TableError::MissingRow));
    assert!(published(&journal).is_empty());
    case.close().await;
}

#[tokio::test]
async fn table_client_delete_returns_the_row_and_publishes() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    let created = table
        .create(create_data("web", at(1_772_600_767_000_000)))
        .await
        .unwrap();
    let deleted = table.delete(&created.search_tool_id).await.unwrap();
    assert_eq!(deleted, created);
    assert_eq!(row_count(&case.store).await, 0);
    assert_eq!(published(&journal), [PUBLISHED, PUBLISHED]);
    case.close().await;
}

#[tokio::test]
async fn table_client_delete_of_a_missing_row_fails_without_publishing() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    let error = table.delete("missing").await.unwrap_err();
    assert!(matches!(error, TableError::MissingRow));
    assert!(published(&journal).is_empty());
    case.close().await;
}

#[tokio::test]
async fn table_client_duplicate_name_is_a_unique_violation_without_publishing() {
    let case = Case::new().await;
    let journal = Journal::default();
    let table = PostgresSearchTools::new(Some(case.client()), recording_publisher(&journal));
    table
        .create(create_data("web", at(1_772_600_767_000_000)))
        .await
        .unwrap();
    let error = table
        .create(create_data("web", at(1_772_600_768_000_000)))
        .await
        .unwrap_err();
    assert!(matches!(&error, TableError::Database(native) if is_unique_violation(native)));
    assert_eq!(published(&journal).len(), 1);
    case.close().await;
}

#[tokio::test]
async fn table_client_reports_a_publish_failure_after_the_row_is_written() {
    let case = Case::new().await;
    let table = PostgresSearchTools::new(Some(case.client()), failing_publisher());
    let error = table
        .create(create_data("web", at(1_772_600_767_000_000)))
        .await
        .unwrap_err();
    assert!(matches!(error, TableError::Publish { .. }));
    assert_eq!(row_count(&case.store).await, 1);
    case.close().await;
}

#[tokio::test]
async fn table_client_without_a_connection_fails_every_call_with_the_litellm_text() {
    let table = PostgresSearchTools::new(None, recording_publisher(&Journal::default()));
    let at_time = at(1_772_600_767_000_000);
    let results = [
        table.create(create_data("web", at_time)).await.unwrap_err(),
        table.find_unique("tool-1").await.unwrap_err(),
        table.find_many(None).await.unwrap_err(),
        table
            .update("tool-1", update_data("web", at_time))
            .await
            .unwrap_err(),
        table.delete("tool-1").await.unwrap_err(),
    ];
    assert!(
        results
            .iter()
            .all(|error| error.to_string() == NO_DB_MESSAGE)
    );
}
