#![cfg(test)]

use axum::http::{Method, StatusCode};
use betterlite_auth::auth_identity::roles::Role;
use betterlite_prompts::registry::prompt_endpoint_contracts::records::AMBIGUOUS_PROMPT_DATA_ERROR;
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::PromptRowJson;
use rstest::rstest;
use serde::Deserialize;

use self::fakes::{Harness, TableMode, context, master_key, proxy_admin, viewer};

mod fakes;

const PROMPT_BODY: &str = r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"hello","api_key":"sk-secret"},"prompt_info":{"prompt_type":"db"}}"#;
const AMBIGUOUS_BODY: &str = r#"{"prompt_id":"greeting","litellm_params":{"prompt_id":"greeting","prompt_integration":"dotprompt","prompt_data":{"template":{"content":"hello"}}},"prompt_info":{"prompt_type":"db"}}"#;
const UNSUPPORTED_BODY: &str = r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"unknown"},"prompt_info":{"prompt_type":"db"}}"#;
const PRODUCTION_BODY: &str = r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"},"prompt_info":{"prompt_type":"db","environment":"production"}}"#;

#[derive(Deserialize)]
struct DetailBody {
    detail: String,
}

#[derive(Deserialize)]
struct ValidationBody {
    detail: Vec<ValidationIssue>,
}

#[derive(Deserialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    issue_type: String,
    loc: Vec<String>,
    msg: String,
}

#[derive(Deserialize)]
struct MessageBody {
    message: String,
}

#[derive(Deserialize)]
struct SpecBody {
    prompt_id: String,
    litellm_params: ParamsBody,
    prompt_info: InfoBody,
    version: Option<i64>,
    environment: String,
    created_by: Option<String>,
}

#[derive(Deserialize)]
struct ParamsBody {
    prompt_integration: String,
    api_key: Option<String>,
    dotprompt_content: Option<String>,
}

#[derive(Deserialize)]
struct InfoBody {
    prompt_type: String,
    environment: String,
}

fn spec(body: &str) -> SpecBody {
    serde_json::from_str(body).expect("a prompt spec body")
}

fn detail(body: &str) -> String {
    serde_json::from_str::<DetailBody>(body)
        .expect("a detail body")
        .detail
}

fn stored_text(json: &PromptRowJson) -> &str {
    match json {
        PromptRowJson::Text(text) => text,
        PromptRowJson::Object(_) => "",
    }
}

#[rstest]
#[case::create(Method::POST, "/prompts", PROMPT_BODY, "create")]
#[case::update(Method::PUT, "/prompts/greeting", PROMPT_BODY, "update")]
#[case::patch(Method::PATCH, "/prompts/greeting", "{}", "patch")]
#[case::delete(Method::DELETE, "/prompts/greeting", "", "delete")]
#[tokio::test]
async fn oneshot_writes_reject_non_admins_with_403(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] body: &str,
    #[case] action: &str,
) {
    let harness = Harness::new(viewer(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let (status, response) = harness.send(method, uri, body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        detail(&response),
        format!("Only proxy admins can {action} prompts")
    );
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[rstest]
#[case::create(Method::POST, "/prompts", PROMPT_BODY)]
#[case::update(Method::PUT, "/prompts/greeting", PROMPT_BODY)]
#[case::patch(Method::PATCH, "/prompts/greeting", "{}")]
#[case::delete(Method::DELETE, "/prompts/greeting", "")]
#[tokio::test]
async fn oneshot_writes_answer_500_without_a_database(
    #[case] method: Method,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let harness = Harness::new(master_key(), TableMode::Absent);
    let (status, response) = harness.send(method, uri, body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(detail(&response).starts_with("DB not connected."));
}

#[tokio::test]
async fn oneshot_admin_check_comes_before_the_database_check() {
    let harness = Harness::new(viewer(), TableMode::Absent);
    let (status, _) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn oneshot_a_proxy_admin_role_may_write() {
    let harness = Harness::new(proxy_admin(), TableMode::Working);
    let (status, response) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec(&response).created_by.as_deref(), Some("alice"));
}

#[rstest]
#[case::create(Method::POST, "/prompts")]
#[case::update(Method::PUT, "/prompts/greeting")]
#[tokio::test]
async fn oneshot_writes_reject_ambiguous_prompt_data_with_400(
    #[case] method: Method,
    #[case] uri: &str,
) {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let (status, response) = harness.send(method, uri, AMBIGUOUS_BODY).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(detail(&response), AMBIGUOUS_PROMPT_DATA_ERROR);
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[tokio::test]
async fn oneshot_patch_rejects_ambiguous_prompt_data_only_when_litellm_params_are_sent() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let body = r#"{"litellm_params":{"prompt_id":"greeting","prompt_integration":"dotprompt","prompt_data":{"template":{"content":"hello"}}}}"#;
    let (status, response) = harness.send(Method::PATCH, "/prompts/greeting", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(detail(&response), AMBIGUOUS_PROMPT_DATA_ERROR);
    let info_only = r#"{"prompt_info":{"prompt_type":"db"}}"#;
    let (status, _) = harness
        .send(Method::PATCH, "/prompts/greeting", info_only)
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[rstest]
#[case::missing_field(r#"{"prompt_id":"greeting"}"#)]
#[case::not_json("not json")]
#[case::trailing_data(
    r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}} extra"#
)]
#[tokio::test]
async fn oneshot_create_answers_422_for_a_body_that_does_not_fit(#[case] body: &str) {
    let harness = Harness::new(master_key(), TableMode::Working);
    let (status, _) = harness.send(Method::POST, "/prompts", body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(harness.rows.lock().expect("rows").is_empty());
}

#[tokio::test]
async fn oneshot_create_locates_the_field_a_body_gets_wrong() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let body = r#"{"prompt_id":"greeting","litellm_params":{}}"#;
    let (status, response) = harness.send(Method::POST, "/prompts", body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = serde_json::from_str::<ValidationBody>(&response)
        .expect("a validation body")
        .detail;
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].loc, ["body", "litellm_params"]);
    assert_eq!(issues[0].issue_type, "value_error");
    assert!(issues[0].msg.contains("prompt_integration"));
}

#[tokio::test]
async fn oneshot_create_allocates_version_one_and_initializes_the_registry() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let (status, response) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    assert_eq!(status, StatusCode::OK);
    let created = spec(&response);
    assert_eq!(created.prompt_id, "greeting.v1");
    assert_eq!(created.version, Some(1));
    assert_eq!(created.environment, "development");
    assert_eq!(created.created_by.as_deref(), Some("default_user_id"));
    assert_eq!(created.litellm_params.prompt_integration, "dotprompt");
    assert_eq!(created.prompt_info.prompt_type, "db");
    assert_eq!(harness.recorded_calls(), vec!["added"]);
    let registry = harness.registry.lock().await;
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::development")
    );
}

#[tokio::test]
async fn oneshot_create_allocates_the_next_version_inside_each_environment() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let (_, first) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    let (_, second) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    let (_, production) = harness
        .send(Method::POST, "/prompts", PRODUCTION_BODY)
        .await;
    assert_eq!(spec(&first).prompt_id, "greeting.v1");
    assert_eq!(spec(&second).prompt_id, "greeting.v2");
    let production = spec(&production);
    assert_eq!(production.prompt_id, "greeting.v1");
    assert_eq!(production.environment, "production");
    assert_eq!(production.prompt_info.environment, "production");
}

#[tokio::test]
async fn oneshot_create_keeps_a_version_suffix_in_the_prompt_id() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let body = PROMPT_BODY.replace(r#""prompt_id":"greeting""#, r#""prompt_id":"greeting.v3""#);
    let (status, response) = harness.send(Method::POST, "/prompts", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec(&response).prompt_id, "greeting.v3.v1");
    assert_eq!(
        harness.rows.lock().expect("rows")[0].prompt_id,
        "greeting.v3"
    );
}

#[tokio::test]
async fn oneshot_create_stores_the_key_in_the_row_and_masks_it_in_the_response() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let (_, response) = harness.send(Method::POST, "/prompts", PROMPT_BODY).await;
    assert!(!response.contains("sk-secret"));
    let created = spec(&response);
    assert_eq!(created.litellm_params.api_key.as_deref(), Some("********"));
    assert_eq!(
        created.litellm_params.dotprompt_content.as_deref(),
        Some("hello")
    );
    let rows = harness.rows.lock().expect("rows");
    let stored = stored_text(&rows[0].litellm_params);
    assert_eq!(
        stored,
        r#"{"prompt_id":null,"prompt_integration":"dotprompt","api_base":null,"api_key":"sk-secret","provider_specific_query_params":null,"ignore_prompt_manager_model":false,"ignore_prompt_manager_optional_params":false,"dotprompt_content":"hello"}"#
    );
}

#[tokio::test]
async fn oneshot_create_stores_db_prompt_info_when_the_request_has_none() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let body = r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}}"#;
    let (status, response) = harness.send(Method::POST, "/prompts", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec(&response).prompt_info.prompt_type, "db");
    let rows = harness.rows.lock().expect("rows");
    let info = rows[0].prompt_info.as_ref().expect("prompt_info");
    assert_eq!(
        stored_text(info),
        r#"{"prompt_type":"db","environment":"development"}"#
    );
}

#[tokio::test]
async fn oneshot_create_keeps_the_row_when_the_registry_cannot_initialize_it() {
    let harness = Harness::new(master_key(), TableMode::Working);
    let (status, response) = harness
        .send(Method::POST, "/prompts", UNSUPPORTED_BODY)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail(&response), "Unsupported prompt: unknown");
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
    assert!(harness.registry.lock().await.in_memory_prompts.is_empty());
}

#[tokio::test]
async fn oneshot_update_answers_404_when_no_version_exists() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("other", 1, "development");
    let (status, response) = harness
        .send(Method::PUT, "/prompts/greeting", PROMPT_BODY)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&response), "Prompt with ID greeting not found");
}

#[tokio::test]
async fn oneshot_update_rejects_config_prompts_with_400() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "config")
        .await;
    let (status, response) = harness
        .send(Method::PUT, "/prompts/greeting", PROMPT_BODY)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(detail(&response), "Cannot update config prompts.");
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[rstest]
#[case::base_id("greeting")]
#[case::dot_suffix("greeting.v1")]
#[case::underscore_suffix("greeting_v1")]
#[tokio::test]
async fn oneshot_update_strips_the_version_suffix_and_creates_the_next_version(
    #[case] path_id: &str,
) {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let uri = format!("/prompts/{path_id}");
    let (status, response) = harness.send(Method::PUT, &uri, PROMPT_BODY).await;
    assert_eq!(status, StatusCode::OK);
    let updated = spec(&response);
    assert_eq!(updated.prompt_id, "greeting.v2");
    assert_eq!(updated.version, Some(2));
    let rows = harness.rows.lock().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].prompt_id, "greeting");
    drop(rows);
    let registry = harness.registry.lock().await;
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v2::development")
    );
}

#[tokio::test]
async fn oneshot_update_allocates_versions_in_the_requested_environment() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness.seed_row("greeting", 2, "development");
    let (_, production) = harness
        .send(Method::PUT, "/prompts/greeting", PRODUCTION_BODY)
        .await;
    let (_, development) = harness
        .send(Method::PUT, "/prompts/greeting", PROMPT_BODY)
        .await;
    let production = spec(&production);
    assert_eq!(production.prompt_id, "greeting.v1");
    assert_eq!(production.environment, "production");
    assert_eq!(spec(&development).prompt_id, "greeting.v3");
}

#[tokio::test]
async fn oneshot_update_keeps_the_row_when_the_registry_cannot_initialize_it() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let (status, _) = harness
        .send(Method::PUT, "/prompts/greeting", UNSUPPORTED_BODY)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(harness.rows.lock().expect("rows").len(), 2);
}

#[tokio::test]
async fn oneshot_patch_answers_404_for_a_missing_row_and_defaults_to_development() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "production");
    let (status, response) = harness.send(Method::PATCH, "/prompts/greeting", "{}").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        detail(&response),
        "Prompt with ID greeting not found in environment development"
    );
    let (status, _) = harness
        .send(
            Method::PATCH,
            "/prompts/greeting?environment=production",
            "{}",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn oneshot_patch_rejects_config_prompts_with_400() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "config")
        .await;
    let (status, response) = harness.send(Method::PATCH, "/prompts/greeting", "{}").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(detail(&response), "Cannot update config prompts.");
}

#[rstest]
#[case::latest_without_suffix("greeting", "greeting.v3")]
#[case::dot_suffix("greeting.v2", "greeting.v2")]
#[case::underscore_suffix("greeting_v1", "greeting.v1")]
#[tokio::test]
async fn oneshot_patch_targets_one_row_by_version_and_changes_no_other(
    #[case] path_id: &str,
    #[case] patched_id: &str,
) {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness.seed_row("greeting", 2, "development");
    harness.seed_row("greeting", 3, "development");
    let uri = format!("/prompts/{path_id}");
    let body = r#"{"litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"new"}}"#;
    let (status, response) = harness.send(Method::PATCH, &uri, body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec(&response).prompt_id, patched_id);
    let rows = harness.rows.lock().expect("rows");
    let changed: Vec<i64> = rows
        .iter()
        .filter(|row| stored_text(&row.litellm_params).contains(r#""dotprompt_content":"new""#))
        .map(|row| row.version)
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(format!("greeting.v{}", changed[0]), patched_id);
}

#[tokio::test]
async fn oneshot_patch_keeps_the_stored_value_of_a_field_the_request_leaves_out() {
    let harness = Harness::new(proxy_admin(), TableMode::Working);
    harness.seed_row_with("greeting", 1, "development", "stored");
    let info = r#"{"prompt_info":{"prompt_type":"db","environment":"development","owner":"team"}}"#;
    let (_, response) = harness.send(Method::PATCH, "/prompts/greeting", info).await;
    let patched = spec(&response);
    assert_eq!(
        patched.litellm_params.dotprompt_content.as_deref(),
        Some("stored")
    );
    assert!(response.contains(r#""owner":"team""#));
    let params =
        r#"{"litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"fresh"}}"#;
    let (_, response) = harness
        .send(Method::PATCH, "/prompts/greeting", params)
        .await;
    let patched = spec(&response);
    assert_eq!(
        patched.litellm_params.dotprompt_content.as_deref(),
        Some("fresh")
    );
    assert!(response.contains(r#""owner":"team""#));
    assert_eq!(patched.created_by.as_deref(), Some("alice"));
}

#[tokio::test]
async fn oneshot_patch_records_the_master_key_user_as_created_by() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let (_, response) = harness.send(Method::PATCH, "/prompts/greeting", "{}").await;
    assert_eq!(
        spec(&response).created_by.as_deref(),
        Some("default_user_id")
    );
}

#[tokio::test]
async fn oneshot_patch_leaves_created_by_alone_for_an_admin_without_a_user() {
    let harness = Harness::new(
        context("service", vec![Role::PlatformAdmin], None),
        TableMode::Working,
    );
    harness.seed_row("greeting", 1, "development");
    let (_, response) = harness.send(Method::PATCH, "/prompts/greeting", "{}").await;
    assert_eq!(spec(&response).created_by.as_deref(), Some("seeder"));
}

#[tokio::test]
async fn oneshot_patch_reloads_the_prompt_in_the_registry() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "db")
        .await;
    let body =
        r#"{"litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"reloaded"}}"#;
    let (status, _) = harness.send(Method::PATCH, "/prompts/greeting", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.recorded_calls(), vec!["removed", "added"]);
    let registry = harness.registry.lock().await;
    let reloaded = registry
        .in_memory_prompts
        .get("greeting.v1::development")
        .expect("reloaded prompt");
    assert_eq!(
        reloaded.litellm_params.dotprompt_content.as_deref(),
        Some("reloaded")
    );
}

#[tokio::test]
async fn oneshot_patch_keeps_the_database_update_when_the_reload_fails() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let body = r#"{"litellm_params":{"prompt_integration":"unknown"}}"#;
    let (status, response) = harness.send(Method::PATCH, "/prompts/greeting", body).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail(&response), "Unsupported prompt: unknown");
    let rows = harness.rows.lock().expect("rows");
    assert!(stored_text(&rows[0].litellm_params).contains(r#""prompt_integration":"unknown""#));
}

#[tokio::test]
async fn oneshot_delete_answers_404_when_the_registry_does_not_know_the_prompt() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    let (status, response) = harness.send(Method::DELETE, "/prompts/greeting", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&response), "Prompt with ID greeting not found");
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[tokio::test]
async fn oneshot_delete_rejects_config_prompts_with_400() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "config")
        .await;
    let (status, response) = harness.send(Method::DELETE, "/prompts/greeting", "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(detail(&response), "Cannot delete config prompts.");
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[rstest]
#[case::base_id("greeting")]
#[case::dot_suffix("greeting.v2")]
#[tokio::test]
async fn oneshot_delete_removes_every_row_of_the_base_id_and_syncs_the_registry(
    #[case] path_id: &str,
) {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness.seed_row("greeting", 2, "development");
    harness.seed_row("greeting", 1, "production");
    harness.seed_row("other", 1, "development");
    harness
        .seed_registry("greeting", 2, "development", "db")
        .await;
    harness
        .seed_registry("greeting", 1, "production", "db")
        .await;
    let uri = format!("/prompts/{path_id}");
    let (status, response) = harness.send(Method::DELETE, &uri, "").await;
    assert_eq!(status, StatusCode::OK);
    let body: MessageBody = serde_json::from_str(&response).expect("message");
    assert_eq!(body.message, "Prompt greeting deleted successfully");
    let rows = harness.rows.lock().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].prompt_id, "other");
    drop(rows);
    assert!(harness.registry.lock().await.in_memory_prompts.is_empty());
    assert_eq!(harness.recorded_calls(), vec!["removed", "removed"]);
}

#[tokio::test]
async fn oneshot_delete_scopes_rows_and_registry_to_the_requested_environment() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness.seed_row("greeting", 1, "production");
    harness
        .seed_registry("greeting", 1, "development", "db")
        .await;
    harness
        .seed_registry("greeting", 1, "production", "db")
        .await;
    let (status, response) = harness
        .send(
            Method::DELETE,
            "/prompts/greeting?environment=production",
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let body: MessageBody = serde_json::from_str(&response).expect("message");
    assert_eq!(
        body.message,
        "Prompt greeting deleted successfully from production"
    );
    let rows = harness.rows.lock().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].environment, "development");
    drop(rows);
    let registry = harness.registry.lock().await;
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::development")
    );
    assert!(
        !registry
            .in_memory_prompts
            .contains_key("greeting.v1::production")
    );
}

#[tokio::test]
async fn oneshot_delete_answers_404_for_an_environment_the_registry_lacks() {
    let harness = Harness::new(master_key(), TableMode::Working);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "db")
        .await;
    let (status, _) = harness
        .send(Method::DELETE, "/prompts/greeting?environment=staging", "")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(harness.rows.lock().expect("rows").len(), 1);
}

#[tokio::test]
async fn oneshot_delete_leaves_the_registry_alone_when_the_database_delete_fails() {
    let harness = Harness::new(master_key(), TableMode::FailingDeletes);
    harness.seed_row("greeting", 1, "development");
    harness
        .seed_registry("greeting", 1, "development", "db")
        .await;
    let (status, response) = harness.send(Method::DELETE, "/prompts/greeting", "").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail(&response), "the fake table failed");
    let registry = harness.registry.lock().await;
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting.v1::development")
    );
}
