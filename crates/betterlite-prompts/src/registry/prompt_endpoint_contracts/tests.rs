#![cfg(test)]

use std::future::{Future, ready};
use std::io;
use std::sync::Mutex;

use betterlite_models::prompt_spec::PromptLiteLLMParams;
use chrono::{DateTime, Utc};
use rstest::rstest;

use super::records::{
    AMBIGUOUS_PROMPT_DATA_ERROR, PatchPromptRequest, Prompt, is_ambiguous_keyed_prompt_data,
};
use super::rows::{PromptRow, PromptRowData, PromptRowJson};
use super::spec_conversion::{
    PromptRowConversionError, create_versioned_prompt_spec, get_latest_prompt_versions,
};
use super::table::{
    PromptRowFilter, PromptRowQuery, PromptRowUpdate, PromptTableActions, PromptVersionError,
    get_next_version_for_prompt,
};
use crate::tests::{json_value, prompt_params, prompt_spec};

fn fixed_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

fn prompt_row(prompt_id: &str, version: i64, environment: &str) -> PromptRow {
    PromptRow {
        id: format!("{prompt_id}-{version}-{environment}"),
        prompt_id: prompt_id.to_owned(),
        version,
        environment: environment.to_owned(),
        created_by: Some("user-1".to_owned()),
        created_at: fixed_time(),
        updated_at: fixed_time(),
        litellm_params: PromptRowJson::Text(
            r#"{"prompt_id":"greeting","prompt_integration":"dotprompt"}"#.to_owned(),
        ),
        prompt_info: Some(PromptRowJson::Text(r#"{"prompt_type":"db"}"#.to_owned())),
    }
}

struct FakePromptTable {
    rows: Vec<PromptRow>,
    failure: Option<&'static str>,
    queries: Mutex<Vec<PromptRowQuery>>,
}

impl FakePromptTable {
    fn answering(rows: Vec<PromptRow>) -> Self {
        Self {
            rows,
            failure: None,
            queries: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: &'static str) -> Self {
        Self {
            rows: Vec::new(),
            failure: Some(message),
            queries: Mutex::new(Vec::new()),
        }
    }
}

impl PromptTableActions for FakePromptTable {
    type Failure = io::Error;

    fn find_many(
        &self,
        query: PromptRowQuery,
    ) -> impl Future<Output = Result<Vec<PromptRow>, Self::Failure>> + Send {
        self.queries.lock().unwrap().push(query);
        let result = self.failure.map_or_else(
            || Ok(self.rows.clone()),
            |message| Err(io::Error::other(message)),
        );
        ready(result)
    }

    fn create(
        &self,
        _data: PromptRowData,
    ) -> impl Future<Output = Result<PromptRow, Self::Failure>> + Send {
        ready(Err(io::Error::other("create is not scripted")))
    }

    fn update(
        &self,
        _filter: PromptRowFilter,
        _data: PromptRowUpdate,
    ) -> impl Future<Output = Result<Option<PromptRow>, Self::Failure>> + Send {
        ready(Err(io::Error::other("update is not scripted")))
    }

    fn delete_many(
        &self,
        _filter: PromptRowFilter,
    ) -> impl Future<Output = Result<u64, Self::Failure>> + Send {
        ready(Err(io::Error::other("delete is not scripted")))
    }
}

#[test]
fn get_latest_prompt_versions_keeps_the_highest_version_per_environment() {
    let prompts = vec![
        prompt_spec("greeting.v1", Some(1), "development"),
        prompt_spec("greeting.v2", Some(2), "development"),
        prompt_spec("greeting.v4", Some(4), "production"),
        prompt_spec("other.v1", Some(1), "development"),
    ];
    let latest = get_latest_prompt_versions(prompts);
    let prompt_ids: Vec<&str> = latest
        .iter()
        .map(|prompt| prompt.prompt_id.as_str())
        .collect();
    assert_eq!(prompt_ids, ["greeting.v2", "other.v1", "greeting.v4"]);
}

#[test]
fn get_latest_prompt_versions_reads_the_version_from_the_id_suffix() {
    let prompts = vec![
        prompt_spec("greeting.v1", Some(9), "development"),
        prompt_spec("greeting.v2", Some(2), "development"),
    ];
    let latest = get_latest_prompt_versions(prompts);
    assert_eq!(latest.len(), 1);
    assert_eq!(latest.first().expect("one prompt").prompt_id, "greeting.v2");
}

#[test]
fn get_latest_prompt_versions_returns_nothing_for_no_prompts() {
    assert!(get_latest_prompt_versions(Vec::new()).is_empty());
}

#[tokio::test]
async fn get_next_version_for_prompt_returns_the_max_version_plus_one() {
    let table = FakePromptTable::answering(vec![
        prompt_row("greeting", 1, "development"),
        prompt_row("greeting", 3, "development"),
    ]);
    let next_version = get_next_version_for_prompt(&table, "greeting", Some("development"))
        .await
        .expect("version allocated");
    assert_eq!(next_version, 4);
}

#[tokio::test]
async fn get_next_version_for_prompt_returns_one_without_matching_rows() {
    let table = FakePromptTable::answering(Vec::new());
    let next_version = get_next_version_for_prompt(&table, "greeting", Some("development"))
        .await
        .expect("version allocated");
    assert_eq!(next_version, 1);
}

#[tokio::test]
async fn get_next_version_for_prompt_queries_the_prompt_id_and_environment() {
    let table = FakePromptTable::answering(Vec::new());
    get_next_version_for_prompt(&table, "greeting", Some("production"))
        .await
        .expect("version allocated");
    let queries = table.queries.lock().unwrap();
    assert_eq!(queries.len(), 1);
    assert_eq!(
        queries[0],
        PromptRowQuery {
            filter: PromptRowFilter {
                id: None,
                prompt_id: Some("greeting".to_owned()),
                version: None,
                environment: Some("production".to_owned()),
            },
            order: Vec::new(),
            take: None,
            distinct: Vec::new(),
        }
    );
}

#[tokio::test]
async fn get_next_version_for_prompt_defaults_the_environment_to_development() {
    let table = FakePromptTable::answering(Vec::new());
    get_next_version_for_prompt(&table, "greeting", None)
        .await
        .expect("version allocated");
    let queries = table.queries.lock().unwrap();
    assert_eq!(
        queries[0].filter.environment,
        Some("development".to_owned())
    );
}

#[tokio::test]
async fn get_next_version_for_prompt_reports_a_table_failure() {
    let table = FakePromptTable::failing("database is down");
    let error = get_next_version_for_prompt(&table, "greeting", Some("development"))
        .await
        .expect_err("query fails");
    assert!(matches!(error, PromptVersionError::Table { .. }));
}

#[tokio::test]
async fn get_next_version_for_prompt_rejects_a_version_that_cannot_be_incremented() {
    let table = FakePromptTable::answering(vec![prompt_row("greeting", i64::MAX, "development")]);
    let error = get_next_version_for_prompt(&table, "greeting", Some("development"))
        .await
        .expect_err("version overflows");
    assert!(matches!(error, PromptVersionError::VersionOverflow));
}

#[tokio::test]
async fn the_prompt_table_actions_expose_every_table_operation() {
    let table = FakePromptTable::answering(Vec::new());
    let data: PromptRowData =
        serde_json::from_str(r#"{"prompt_id":"greeting"}"#).expect("row data parses");
    let created = table.create(data).await;
    let update = PromptRowUpdate {
        litellm_params: PromptRowJson::Text("{}".to_owned()),
        prompt_info: PromptRowJson::Text("{}".to_owned()),
        created_by: None,
    };
    let updated = table.update(PromptRowFilter::default(), update).await;
    let deleted = table.delete_many(PromptRowFilter::default()).await;
    assert_eq!(
        created.expect_err("create fails").to_string(),
        "create is not scripted"
    );
    assert_eq!(
        updated.expect_err("update fails").to_string(),
        "update is not scripted"
    );
    assert_eq!(
        deleted.expect_err("delete fails").to_string(),
        "delete is not scripted"
    );
}

#[test]
fn create_versioned_prompt_spec_builds_the_versioned_prompt_id() {
    let row = prompt_row("greeting", 2, "production");
    let spec = create_versioned_prompt_spec(&row).expect("row converts");
    assert_eq!(spec.prompt_id, "greeting.v2");
    assert_eq!(spec.version, Some(2));
}

#[test]
fn create_versioned_prompt_spec_reads_stored_json_text() {
    let row = prompt_row("greeting", 2, "production");
    let spec = create_versioned_prompt_spec(&row).expect("row converts");
    assert_eq!(spec.litellm_params.prompt_id.as_deref(), Some("greeting"));
    assert_eq!(spec.litellm_params.prompt_integration, "dotprompt");
    assert_eq!(spec.prompt_info.prompt_type, "db");
}

#[test]
fn create_versioned_prompt_spec_reads_stored_json_objects() {
    let mut row = prompt_row("greeting", 2, "production");
    row.litellm_params = PromptRowJson::Object(
        serde_json::from_str(r#"{"prompt_integration":"custom","temperature":0.2}"#)
            .expect("params object parses"),
    );
    row.prompt_info = Some(PromptRowJson::Object(
        serde_json::from_str(r#"{"prompt_type":"config"}"#).expect("info object parses"),
    ));
    let spec = create_versioned_prompt_spec(&row).expect("row converts");
    assert_eq!(spec.litellm_params.prompt_id, None);
    assert_eq!(spec.litellm_params.prompt_integration, "custom");
    assert_eq!(
        spec.litellm_params.extra.get("temperature"),
        Some(&json_value("0.2"))
    );
    assert_eq!(spec.prompt_info.prompt_type, "config");
}

#[test]
fn create_versioned_prompt_spec_defaults_missing_prompt_info_to_database() {
    let mut row = prompt_row("greeting", 2, "production");
    row.prompt_info = None;
    let spec = create_versioned_prompt_spec(&row).expect("row converts");
    assert_eq!(spec.prompt_info.prompt_type, "db");
    assert_eq!(spec.prompt_info.environment, "development");
    assert!(spec.prompt_info.extra.is_empty());
}

#[test]
fn create_versioned_prompt_spec_keeps_the_row_metadata() {
    let row = prompt_row("greeting", 2, "production");
    let spec = create_versioned_prompt_spec(&row).expect("row converts");
    assert_eq!(spec.environment, "production");
    assert_eq!(spec.created_by.as_deref(), Some("user-1"));
    assert_eq!(spec.created_at, Some(fixed_time()));
    assert_eq!(spec.updated_at, Some(fixed_time()));
}

#[test]
fn create_versioned_prompt_spec_rejects_unparsable_litellm_params() {
    let mut row = prompt_row("greeting", 2, "production");
    row.litellm_params = PromptRowJson::Text("not json".to_owned());
    let error = create_versioned_prompt_spec(&row).expect_err("params do not parse");
    assert!(matches!(
        error,
        PromptRowConversionError::InvalidStoredJson { .. }
    ));
}

#[test]
fn create_versioned_prompt_spec_rejects_litellm_params_of_the_wrong_shape() {
    let mut row = prompt_row("greeting", 2, "production");
    row.litellm_params = PromptRowJson::Text("[]".to_owned());
    let error = create_versioned_prompt_spec(&row).expect_err("params do not parse");
    assert!(matches!(
        error,
        PromptRowConversionError::InvalidStoredJson { .. }
    ));
}

#[test]
fn create_versioned_prompt_spec_rejects_unparsable_prompt_info() {
    let mut row = prompt_row("greeting", 2, "production");
    row.prompt_info = Some(PromptRowJson::Text("not json".to_owned()));
    let error = create_versioned_prompt_spec(&row).expect_err("prompt info does not parse");
    assert!(matches!(
        error,
        PromptRowConversionError::InvalidStoredJson { .. }
    ));
}

fn ambiguous_params(prompt_id: Option<&str>, prompt_data: Option<&str>) -> PromptLiteLLMParams {
    let mut params = prompt_params();
    params.prompt_id = prompt_id.map(str::to_owned);
    if let Some(text) = prompt_data {
        params
            .extra
            .insert("prompt_data".to_owned(), json_value(text));
    }
    params
}

#[rstest]
#[case::keyed_data_with_prompt_id(
    ambiguous_params(Some("greeting"), Some(r#"{"first":{"content":"hello"}}"#)),
    true
)]
#[case::flat_data_with_prompt_id(
    ambiguous_params(Some("greeting"), Some(r#"{"content":"hello","metadata":{}}"#)),
    false
)]
#[case::keyed_data_without_prompt_id(
    ambiguous_params(None, Some(r#"{"first":{"content":"hello"}}"#)),
    false
)]
#[case::keyed_data_with_empty_prompt_id(
    ambiguous_params(Some(""), Some(r#"{"first":{"content":"hello"}}"#)),
    true
)]
#[case::missing_prompt_data(ambiguous_params(Some("greeting"), None), false)]
#[case::empty_prompt_data(ambiguous_params(Some("greeting"), Some("{}")), false)]
#[case::prompt_data_that_is_not_an_object(
    ambiguous_params(Some("greeting"), Some(r#"["content"]"#)),
    false
)]
#[case::prompt_data_that_is_a_string(
    ambiguous_params(Some("greeting"), Some(r#""content""#)),
    false
)]
fn is_ambiguous_keyed_prompt_data_checks_the_prompt_data_shape(
    #[case] params: PromptLiteLLMParams,
    #[case] expected: bool,
) {
    assert_eq!(is_ambiguous_keyed_prompt_data(&params), expected);
}

#[test]
fn the_ambiguous_prompt_data_error_matches_litellm() {
    assert_eq!(
        AMBIGUOUS_PROMPT_DATA_ERROR,
        "litellm_params.prompt_id cannot be combined with prompt_data keyed by template name. Send a flat template, prompt_data={\"content\": \"...\", \"metadata\": {...}}, together with litellm_params.prompt_id, or send prompt_data={\"<template_id>\": {\"content\": \"...\", \"metadata\": {...}}} without litellm_params.prompt_id."
    );
}

#[test]
fn prompt_deserializes_with_an_optional_prompt_info() {
    let prompt: Prompt = serde_json::from_str(
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"},"prompt_info":{"prompt_type":"db"}}"#,
    )
    .expect("prompt parses");
    assert_eq!(prompt.prompt_id, "greeting");
    assert_eq!(prompt.litellm_params.prompt_integration, "dotprompt");
    assert_eq!(prompt.prompt_info.expect("prompt info").prompt_type, "db");
}

#[test]
fn prompt_defaults_prompt_info_to_none() {
    let prompt: Prompt = serde_json::from_str(
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}}"#,
    )
    .expect("prompt parses");
    assert!(prompt.prompt_info.is_none());
}

#[test]
fn patch_prompt_request_deserializes_without_fields() {
    let request: PatchPromptRequest = serde_json::from_str("{}").expect("request parses");
    assert!(request.litellm_params.is_none());
    assert!(request.prompt_info.is_none());
}

#[test]
fn patch_prompt_request_deserializes_each_field_on_its_own() {
    let request: PatchPromptRequest =
        serde_json::from_str(r#"{"prompt_info":{"prompt_type":"db"}}"#).expect("request parses");
    assert!(request.litellm_params.is_none());
    assert_eq!(request.prompt_info.expect("prompt info").prompt_type, "db");
}

#[test]
fn patch_prompt_request_deserializes_litellm_params() {
    let request: PatchPromptRequest =
        serde_json::from_str(r#"{"litellm_params":{"prompt_integration":"custom"}}"#)
            .expect("request parses");
    assert_eq!(
        request.litellm_params.expect("params").prompt_integration,
        "custom"
    );
    assert!(request.prompt_info.is_none());
}

#[test]
fn prompt_row_data_defaults_version_and_environment() {
    let data: PromptRowData =
        serde_json::from_str(r#"{"prompt_id":"greeting"}"#).expect("row data parses");
    assert_eq!(data.prompt_id, "greeting");
    assert_eq!(data.version, 1);
    assert_eq!(data.environment, "development");
    assert!(data.created_by.is_none());
    assert!(data.litellm_params.is_none());
    assert!(data.prompt_info.is_none());
    assert!(data.created_at.is_none());
    assert!(data.updated_at.is_none());
}

#[test]
fn prompt_row_data_reads_every_declared_field() {
    let data: PromptRowData = serde_json::from_str(
        r#"{
            "prompt_id": "greeting",
            "version": 3,
            "environment": "production",
            "created_by": "user-2",
            "litellm_params": {"prompt_integration": "dotprompt"},
            "prompt_info": {"prompt_type": "db"},
            "created_at": "2023-11-14T22:13:20Z",
            "updated_at": "2023-11-14T22:13:20Z"
        }"#,
    )
    .expect("row data parses");
    assert_eq!(data.version, 3);
    assert_eq!(data.environment, "production");
    assert_eq!(data.created_by.as_deref(), Some("user-2"));
    assert!(data.litellm_params.is_some());
    assert!(data.prompt_info.is_some());
    assert_eq!(data.created_at, Some(fixed_time()));
    assert_eq!(data.updated_at, Some(fixed_time()));
}

#[test]
fn prompt_row_round_trips_through_json() {
    let row = prompt_row("greeting", 2, "production");
    let text = serde_json::to_string(&row).expect("row serializes");
    let parsed: PromptRow = serde_json::from_str(&text).expect("row parses");
    assert_eq!(parsed, row);
}

#[test]
fn prompt_row_reads_json_object_fields() {
    let row: PromptRow = serde_json::from_str(
        r#"{
            "id": "row-1",
            "prompt_id": "greeting",
            "version": 1,
            "environment": "development",
            "created_by": null,
            "created_at": "2023-11-14T22:13:20Z",
            "updated_at": "2023-11-14T22:13:20Z",
            "litellm_params": {"prompt_integration": "dotprompt"},
            "prompt_info": null
        }"#,
    )
    .expect("row parses");
    assert!(matches!(row.litellm_params, PromptRowJson::Object(_)));
    assert!(row.prompt_info.is_none());
    assert!(row.created_by.is_none());
}
