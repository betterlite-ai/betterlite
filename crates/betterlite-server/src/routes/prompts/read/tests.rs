#![cfg(test)]

mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowOrder,
};
use rstest::rstest;
use serde_json::json;

use super::VERSIONS_ADMIN_ONLY;
use support::{
    FakePromptTable, call, config_prompt, dotprompt_prompt, key_context, keyed_prompt,
    master_context, ordinary_context, registry, role_context, state, stored_prompt,
};

#[tokio::test]
async fn oneshot_list_metadata_allowlist_narrows_to_the_named_prompt() {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha.v2", "development"),
        config_prompt("beta", "development"),
    ]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/list",
        key_context(r#"{"metadata": {"prompts": ["alpha"]}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"].as_array().unwrap().len(), 1);
    assert_eq!(body["prompts"][0]["prompt_id"], "alpha");
    assert!(body["prompts"][0]["version"].is_null());
}

#[rstest]
#[case::base_id("alpha")]
#[case::versioned_id("alpha.v2")]
#[tokio::test]
async fn oneshot_list_metadata_allowlist_accepts_the_base_or_versioned_id(#[case] allowed: &str) {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha.v2", "development"),
    ]);
    let key = key_context(&json!({"metadata": {"prompts": [allowed]}}).to_string());
    let (status, body) = call(state(prompts, None), "/prompts/list", key).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"].as_array().unwrap().len(), 1);
    assert_eq!(body["prompts"][0]["prompt_id"], "alpha");
}

#[rstest]
#[case::admin(&[Role::PlatformAdmin])]
#[case::viewer(&[Role::PlatformViewer])]
#[tokio::test]
async fn oneshot_list_admin_keeps_the_latest_version_per_environment(#[case] roles: &[Role]) {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha.v2", "development"),
        config_prompt("alpha_v3", "production"),
    ]);
    let (status, body) = call(state(prompts, None), "/prompts/list", role_context(roles)).await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["prompts"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["prompt_id"], "alpha");
    assert_eq!(listed[0]["environment"], "development");
    assert_eq!(listed[1]["prompt_id"], "alpha");
    assert_eq!(listed[1]["environment"], "production");
}

#[tokio::test]
async fn oneshot_list_environment_query_keeps_only_that_environment() {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha_v3", "production"),
    ]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/list?environment=production",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"].as_array().unwrap().len(), 1);
    assert_eq!(body["prompts"][0]["environment"], "production");
}

#[rstest]
#[case::no_metadata("{}")]
#[case::null_allowlist(r#"{"metadata": {"prompts": null}}"#)]
#[tokio::test]
async fn oneshot_list_without_an_allowlist_is_empty_for_an_ordinary_key(#[case] key_fields: &str) {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/list",
        key_context(key_fields),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"], json!([]));
}

#[tokio::test]
async fn oneshot_list_master_key_sees_every_latest_prompt() {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha.v2", "development"),
        config_prompt("beta", "staging"),
    ]);
    let (status, body) = call(state(prompts, None), "/prompts/list", master_context()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"].as_array().unwrap().len(), 2);
    assert!(body["prompts"][0]["version"].is_null());
}

#[rstest]
#[case::list("/prompts/list")]
#[case::info("/prompts/alpha/info")]
#[tokio::test]
async fn oneshot_list_and_info_reject_wrong_typed_metadata_prompts(#[case] uri: &str) {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (status, body) = call(
        state(prompts, None),
        uri,
        key_context(r#"{"metadata": {"prompts": "alpha"}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("metadata.prompts")
    );
}

#[tokio::test]
async fn oneshot_versions_admin_reads_the_history_newest_first() {
    let table = FakePromptTable::answering(vec![
        stored_prompt("alpha", 1, "development"),
        stored_prompt("alpha", 2, "development"),
        stored_prompt("alpha", 3, "production"),
    ]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(Arc::clone(&table))),
        "/prompts/alpha/versions",
        role_context(&[Role::PlatformAdmin]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["prompts"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0]["version"], json!(3));
    assert_eq!(listed[1]["version"], json!(2));
    assert_eq!(listed[2]["version"], json!(1));
    assert_eq!(listed[0]["prompt_id"], "alpha");
    let queries = table.queries();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].filter.prompt_id.as_deref(), Some("alpha"));
    assert_eq!(
        queries[0].order,
        vec![PromptRowOrder {
            column: PromptColumn::Version,
            descending: true,
        }]
    );
}

#[tokio::test]
async fn oneshot_versions_admin_view_key_reads_the_history() {
    let table = FakePromptTable::answering(vec![stored_prompt("alpha", 1, "development")]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(table)),
        "/prompts/alpha/versions",
        key_context(r#"{"user_role": "proxy_admin"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompts"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn oneshot_versions_ordinary_key_is_forbidden() {
    let table = FakePromptTable::answering(vec![stored_prompt("alpha", 1, "development")]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(Arc::clone(&table))),
        "/prompts/alpha/versions",
        ordinary_context(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["detail"], VERSIONS_ADMIN_ONLY);
    assert!(table.queries().is_empty());
}

#[tokio::test]
async fn oneshot_versions_environment_query_filters_the_history() {
    let table = FakePromptTable::answering(vec![
        stored_prompt("alpha", 1, "development"),
        stored_prompt("alpha", 2, "development"),
        stored_prompt("alpha", 3, "production"),
    ]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(Arc::clone(&table))),
        "/prompts/alpha/versions?environment=development",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["prompts"].as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["version"], json!(2));
    assert_eq!(
        table.queries()[0].filter.environment.as_deref(),
        Some("development")
    );
}

#[tokio::test]
async fn oneshot_versions_fallback_without_a_table_sorts_the_registry() {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha.v2", "development"),
        config_prompt("alpha_v3", "development"),
    ]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/alpha/versions",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = body["prompts"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0]["version"], json!(3));
    assert_eq!(listed[2]["version"], json!(1));
}

#[tokio::test]
async fn oneshot_versions_missing_history_is_a_not_found() {
    let table = FakePromptTable::answering(Vec::new());
    let (status, body) = call(
        state(registry(Vec::new()), Some(table)),
        "/prompts/alpha/versions",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["detail"], "No versions found for prompt ID alpha");
}

#[tokio::test]
async fn oneshot_versions_table_failure_is_a_server_error() {
    let (status, body) = call(
        state(registry(Vec::new()), Some(FakePromptTable::failing())),
        "/prompts/alpha/versions",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["detail"], "the prompt table query failed");
}

#[rstest]
#[case::latest("/prompts/alpha/info?environment=development", None, 2)]
#[case::suffix("/prompts/alpha.v1/info?environment=development", Some(1), 1)]
#[tokio::test]
async fn oneshot_info_environment_selects_the_stored_version(
    #[case] uri: &str,
    #[case] queried: Option<i64>,
    #[case] shown: i64,
) {
    let table = FakePromptTable::answering(vec![
        stored_prompt("alpha", 1, "development"),
        stored_prompt("alpha", 2, "development"),
    ]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(Arc::clone(&table))),
        uri,
        role_context(&[Role::PlatformAdmin]),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompt_spec"]["prompt_id"], "alpha");
    assert_eq!(body["prompt_spec"]["version"], json!(shown));
    let queries = table.queries();
    assert_eq!(queries.len(), 2);
    assert_eq!(queries[1].filter.version, queried);
    assert_eq!(queries[1].take, Some(1));
    assert_eq!(
        queries[1].filter.environment.as_deref(),
        Some("development")
    );
}

#[tokio::test]
async fn oneshot_info_unknown_suffix_falls_back_to_the_registry() {
    let table = FakePromptTable::answering(vec![stored_prompt("alpha", 1, "development")]);
    let (status, body) = call(
        state(registry(Vec::new()), Some(table)),
        "/prompts/alpha.v2/info?environment=development",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        "Prompt alpha.v2 not found in environment development"
    );
}

#[tokio::test]
async fn oneshot_info_environment_picks_the_registry_prompt() {
    let prompts = registry(vec![
        config_prompt("alpha", "development"),
        config_prompt("alpha_v3", "production"),
    ]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/alpha/info?environment=production",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["prompt_spec"]["environment"], "production");
    assert_eq!(body["prompt_spec"]["version"], json!(3));
    assert_eq!(body["environments"], json!([]));
}

#[tokio::test]
async fn oneshot_info_metadata_key_cannot_read_another_prompt() {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let key = key_context(r#"{"user_role": "proxy_admin", "metadata": {"prompts": ["beta"]}}"#);
    let (status, body) = call(state(prompts, None), "/prompts/alpha/info", key).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], "Prompt alpha not found");
}

#[tokio::test]
async fn oneshot_info_ordinary_key_is_forbidden() {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/alpha/info",
        ordinary_context(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["detail"],
        "You are not authorized to access this prompt"
    );
}

#[tokio::test]
async fn oneshot_info_dotprompt_content_becomes_the_raw_template() {
    let prompts = registry(vec![dotprompt_prompt(
        "alpha",
        "development",
        "---\nmodel: gpt-4o\n---\nHello {{name}}",
    )]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/alpha/info",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["raw_prompt_template"]["litellm_prompt_id"], "alpha");
    assert_eq!(body["raw_prompt_template"]["content"], "Hello {{name}}");
    assert_eq!(body["raw_prompt_template"]["metadata"]["model"], "gpt-4o");
    assert_eq!(body["prompt_spec"]["version"], json!(1));
}

#[tokio::test]
async fn oneshot_info_without_dotprompt_content_has_no_template() {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (status, body) = call(
        state(prompts, None),
        "/prompts/alpha/info",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["raw_prompt_template"].is_null());
}

#[tokio::test]
async fn oneshot_info_environments_are_sorted_and_distinct() {
    let table = FakePromptTable::answering(vec![
        stored_prompt("alpha", 1, "production"),
        stored_prompt("alpha", 2, "development"),
        stored_prompt("alpha", 3, "development"),
        stored_prompt("alpha", 4, ""),
    ]);
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (status, body) = call(
        state(prompts, Some(table)),
        "/prompts/alpha/info",
        master_context(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["environments"], json!(["development", "production"]));
}

#[tokio::test]
async fn oneshot_info_and_its_alias_answer_the_same_record() {
    let prompts = registry(vec![config_prompt("alpha", "development")]);
    let (alias_status, alias_body) = call(
        state(Arc::clone(&prompts), None),
        "/prompts/alpha",
        master_context(),
    )
    .await;
    let (info_status, info_body) = call(
        state(prompts, None),
        "/prompts/alpha/info",
        master_context(),
    )
    .await;
    assert_eq!(alias_status, StatusCode::OK);
    assert_eq!(alias_status, info_status);
    assert_eq!(alias_body, info_body);
}

#[rstest]
#[case::plain("/prompts/alpha/info", "Prompt alpha not found")]
#[case::environment(
    "/prompts/alpha/info?environment=production",
    "Prompt alpha not found in environment production"
)]
#[tokio::test]
async fn oneshot_info_missing_prompt_is_a_bad_request(#[case] uri: &str, #[case] detail: &str) {
    let (status, body) = call(state(registry(Vec::new()), None), uri, master_context()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"], detail);
}

#[tokio::test]
async fn oneshot_list_redacts_stored_api_keys() {
    let prompts = registry(vec![keyed_prompt("alpha", "sk-prompt-secret")]);
    let (status, body) = call(state(prompts, None), "/prompts/list", master_context()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["prompts"][0]["litellm_params"]["api_key"],
        crate::cache_settings::REDACTED_VALUE
    );
    assert!(!body.to_string().contains("sk-prompt-secret"));
}
