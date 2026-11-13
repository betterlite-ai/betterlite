#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::budget_permission_rows::JsonData;

use support::*;

#[path = "tests/support.rs"]
mod support;

#[rstest]
#[case::file_object(("file_object", "null", r#"{"id":"f","bytes":3,"created_at":1,"filename":"a.txt","object":"file","purpose":"batch"}"#, r#"{"id":"f","bytes":3,"created_at":1,"filename":"a.txt","object":"file","purpose":"batch"}"#, true))]
#[case::created_by(("created_by", "null", r#""c""#, r#""c""#, true))]
#[case::team_id(("team_id", "null", r#""t""#, r#""t""#, true))]
#[case::updated_by(("updated_by", "null", r#""u""#, r#""u""#, true))]
#[case::storage_backend(("storage_backend", "null", r#""s3""#, r#""s3""#, true))]
#[case::storage_url(("storage_url", "null", r#""s3://bucket/f""#, r#""s3://bucket/f""#, true))]
fn managed_file_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("managed_file", managed_file_base(), spec);
}

#[rstest]
#[case::unified_file_id("unified_file_id")]
#[case::model_mappings("model_mappings")]
#[case::flat_model_file_ids("flat_model_file_ids")]
fn managed_file_required_fields(#[case] field: &str) {
    assert_required_field("managed_file", managed_file_base(), field);
}

#[rstest]
#[case::created_by("created_by")]
#[case::team_id("team_id")]
#[case::org_id("org_id")]
fn managed_object_fields(#[case] field: &str) {
    let mut row = managed_object_base();
    assert_eq!(
        field_of(&dumped("managed_object", &row), field),
        JsonData::Null
    );
    set_field(&mut row, field, data(r#""v""#));
    assert_eq!(
        field_of(&dumped("managed_object", &row), field),
        data(r#""v""#)
    );
    set_field(&mut row, field, JsonData::Null);
    assert!(accepts("managed_object", &row));
}

#[rstest]
#[case::unified_object_id("unified_object_id")]
#[case::model_object_id("model_object_id")]
#[case::file_purpose("file_purpose")]
#[case::file_object("file_object")]
fn managed_object_required_fields(#[case] field: &str) {
    assert_required_field("managed_object", managed_object_base(), field);
}

#[rstest]
#[case::batch("batch", true)]
#[case::fine_tune("fine-tune", true)]
#[case::response("response", true)]
#[case::container("container", true)]
#[case::other("other", false)]
fn managed_purpose_literals(#[case] purpose: &str, #[case] accepted: bool) {
    let mut row = managed_object_base();
    set_field(&mut row, "file_purpose", data(&format!("\"{purpose}\"")));
    assert_eq!(decode_envelope("managed_object", &row).is_ok(), accepted);
}

#[rstest]
#[case::batch_object("batch", BATCH_OBJECT, "completion_window")]
#[case::fine_tuning_object("fine-tune", FINE_TUNING_OBJECT, "hyperparameters")]
#[case::response_object("response", RESPONSE_OBJECT, "output")]
fn embedded_object_union(#[case] purpose: &str, #[case] object: &str, #[case] marker: &str) {
    let mut row = managed_object_base();
    set_field(&mut row, "file_purpose", data(&format!("\"{purpose}\"")));
    set_field(&mut row, "file_object", data(object));
    let dumped = dumped("managed_object", &row);
    assert!(
        field_of(&field_of(&dumped, "file_object"), marker) != JsonData::Null,
        "{marker}"
    );
}

#[test]
fn embedded_object_union_rejects_a_non_object() {
    let mut row = managed_object_base();
    set_field(&mut row, "file_object", data(r#""batch""#));
    assert!(!accepts("managed_object", &row));
    let mut mismatched = managed_object_base();
    set_field(&mut mismatched, "file_purpose", data(r#""container""#));
    set_field(&mut mismatched, "file_object", data(RESPONSE_OBJECT));
    assert!(accepts("managed_object", &mismatched));
}

#[rstest]
#[case::resource_object(("resource_object", "null", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::created_by(("created_by", "null", r#""c""#, r#""c""#, true))]
#[case::team_id(("team_id", "null", r#""t""#, r#""t""#, true))]
#[case::updated_by(("updated_by", "null", r#""u""#, r#""u""#, true))]
#[case::storage_backend(("storage_backend", "null", r#""s3""#, r#""s3""#, true))]
#[case::storage_url(("storage_url", "null", r#""s3://bucket""#, r#""s3://bucket""#, true))]
fn singular_vector_store_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("managed_vector_store", singular_base(), spec);
}

#[rstest]
#[case::unified_resource_id("unified_resource_id")]
#[case::model_mappings("model_mappings")]
#[case::flat_model_resource_ids("flat_model_resource_ids")]
fn singular_vector_store_required_fields(#[case] field: &str) {
    assert_required_field("managed_vector_store", singular_base(), field);
}

#[rstest]
#[case::vector_store_name(("vector_store_name", "null", r#""n""#, r#""n""#, true))]
#[case::vector_store_description(("vector_store_description", "null", r#""d""#, r#""d""#, true))]
#[case::vector_store_metadata(("vector_store_metadata", "null", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::created_at(("created_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::updated_at(("updated_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::litellm_credential_name(("litellm_credential_name", "null", r#""c""#, r#""c""#, true))]
#[case::litellm_params(("litellm_params", "null", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::team_id(("team_id", "null", r#""t""#, r#""t""#, true))]
#[case::user_id(("user_id", "null", r#""u""#, r#""u""#, true))]
#[case::is_config(("is_config", "false", "true", "true", false))]
fn plural_vector_store_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("managed_vector_stores", plural_base(), spec);
}

#[rstest]
#[case::vector_store_id("vector_store_id")]
#[case::custom_llm_provider("custom_llm_provider")]
fn plural_vector_store_required_fields(#[case] field: &str) {
    assert_required_field("managed_vector_stores", plural_base(), field);
}

#[rstest]
#[case::value(("value", r#""""#, r#""v""#, r#""v""#, false))]
#[case::scope(("scope", r#""global""#, r#""user""#, r#""user""#, false))]
#[case::description(("description", "null", r#""d""#, r#""d""#, true))]
fn mcp_env_var_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("mcp_env_var", data(r#"{"name":"A"}"#), spec);
}

#[test]
fn mcp_env_var_requires_name() {
    assert!(!accepts("mcp_env_var", &data("{}")));
    assert!(!accepts("mcp_env_var", &data(r#"{"name":null}"#)));
}

#[rstest]
#[case::is_config(("is_config", "false", "true", "true", false))]
#[case::allow_all_keys(("allow_all_keys", "false", "true", "true", false))]
#[case::delegate_auth_to_upstream(("delegate_auth_to_upstream", "false", "true", "true", false))]
#[case::oauth_passthrough(("oauth_passthrough", "false", "true", "true", false))]
#[case::per_server_oauth_discovery(("per_server_oauth_discovery", "false", "true", "true", false))]
#[case::is_byok(("is_byok", "false", "true", "true", false))]
#[case::available_on_public_internet(("available_on_public_internet", "true", "false", "false", false))]
#[case::teams(("teams", "[]", r#"[{"team_id":"t"}]"#, r#"[{"team_id":"t"}]"#, false))]
#[case::mcp_access_groups(("mcp_access_groups", "[]", r#"["g"]"#, r#"["g"]"#, false))]
#[case::allowed_tools(("allowed_tools", "[]", r#"["t"]"#, r#"["t"]"#, false))]
#[case::extra_headers(("extra_headers", "[]", r#"["h"]"#, r#"["h"]"#, false))]
#[case::args(("args", "[]", r#"["-x"]"#, r#"["-x"]"#, false))]
#[case::byok_description(("byok_description", "[]", r#"["d"]"#, r#"["d"]"#, false))]
#[case::env(("env", "{}", r#"{"A":"B"}"#, r#"{"A":"B"}"#, true))]
#[case::status(("status", r#""unknown""#, r#""healthy""#, r#""healthy""#, true))]
#[case::approval_status(("approval_status", r#""active""#, r#""pending_review""#, r#""pending_review""#, true))]
#[case::server_name(("server_name", "null", r#""n""#, r#""n""#, true))]
#[case::alias(("alias", "null", r#""a""#, r#""a""#, true))]
#[case::description(("description", "null", r#""d""#, r#""d""#, true))]
#[case::url(("url", "null", r#""https://example.test""#, r#""https://example.test""#, true))]
#[case::spec_path(("spec_path", "null", r#""/spec""#, r#""/spec""#, true))]
#[case::auth_type(("auth_type", "null", r#""api_key""#, r#""api_key""#, true))]
#[case::credentials(("credentials", "null", r#"{"auth_value":"v"}"#, r#"{"auth_value":"v"}"#, true))]
#[case::instructions(("instructions", "null", r#""i""#, r#""i""#, true))]
#[case::created_at(("created_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::created_by(("created_by", "null", r#""c""#, r#""c""#, true))]
#[case::updated_at(("updated_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::updated_by(("updated_by", "null", r#""u""#, r#""u""#, true))]
#[case::tool_name_to_display_name(("tool_name_to_display_name", "null", r#"{"t":"d"}"#, r#"{"t":"d"}"#, true))]
#[case::tool_name_to_description(("tool_name_to_description", "null", r#"{"t":"d"}"#, r#"{"t":"d"}"#, true))]
#[case::pinned_tools(("pinned_tools", "null", r#"{"t":{}}"#, r#"{"t":{"description":"","input_schema":{}}}"#, true))]
#[case::mcp_info(("mcp_info", "null", r#"{"protocol_version":"auto"}"#, r#"{"protocol_version":"auto"}"#, true))]
#[case::static_headers(("static_headers", "null", r#"{"A":"B"}"#, r#"{"A":"B"}"#, true))]
#[case::env_vars(("env_vars", "null", r#"[{"name":"A"}]"#, r#"[{"description":null,"name":"A","scope":"global","value":""}]"#, true))]
#[case::last_health_check(("last_health_check", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::health_check_error(("health_check_error", "null", r#""e""#, r#""e""#, true))]
#[case::command(("command", "null", r#""cmd""#, r#""cmd""#, true))]
#[case::issuer(("issuer", "null", r#""iss""#, r#""iss""#, true))]
#[case::authorization_url(("authorization_url", "null", r#""https://a.test""#, r#""https://a.test""#, true))]
#[case::token_url(("token_url", "null", r#""https://t.test""#, r#""https://t.test""#, true))]
#[case::registration_url(("registration_url", "null", r#""https://r.test""#, r#""https://r.test""#, true))]
#[case::oauth2_flow(("oauth2_flow", "null", r#""client_credentials""#, r#""client_credentials""#, true))]
#[case::token_exchange_endpoint(("token_exchange_endpoint", "null", r#""https://x.test""#, r#""https://x.test""#, true))]
#[case::audience(("audience", "null", r#""aud""#, r#""aud""#, true))]
#[case::subject_token_type(("subject_token_type", "null", r#""urn:type""#, r#""urn:type""#, true))]
#[case::token_exchange_profile(("token_exchange_profile", "null", r#""p""#, r#""p""#, true))]
#[case::dcr_bridge(("dcr_bridge", "null", "true", "true", true))]
#[case::byok_api_key_help_url(("byok_api_key_help_url", "null", r#""https://h.test""#, r#""https://h.test""#, true))]
#[case::has_user_credential(("has_user_credential", "null", "true", "true", true))]
#[case::connected_app_reachable(("connected_app_reachable", "null", "false", "false", true))]
#[case::source_url(("source_url", "null", r#""https://s.test""#, r#""https://s.test""#, true))]
#[case::timeout(("timeout", "null", "4", "4.0", true))]
#[case::max_concurrent_requests(("max_concurrent_requests", "null", "4", "4", true))]
#[case::submitted_by(("submitted_by", "null", r#""s""#, r#""s""#, true))]
#[case::submitted_at(("submitted_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::reviewed_at(("reviewed_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::review_notes(("review_notes", "null", r#""notes""#, r#""notes""#, true))]
fn mcp_server_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("mcp_server", mcp_server_base(), spec);
}

#[rstest]
#[case::server_id("server_id")]
#[case::transport("transport")]
fn mcp_server_required_fields(#[case] field: &str) {
    assert_required_field("mcp_server", mcp_server_base(), field);
}

#[rstest]
#[case::source(("source", r#""custom""#, r#""builtin""#, r#""builtin""#, false))]
#[case::display_title(("display_title", "null", r#""t""#, r#""t""#, true))]
#[case::description(("description", "null", r#""d""#, r#""d""#, true))]
#[case::instructions(("instructions", "null", r#""i""#, r#""i""#, true))]
#[case::latest_version(("latest_version", "null", r#""2""#, r#""2""#, true))]
#[case::file_content(("file_content", "null", r#""abc""#, r#""abc""#, true))]
#[case::file_name(("file_name", "null", r#""a.txt""#, r#""a.txt""#, true))]
#[case::file_type(("file_type", "null", r#""text/plain""#, r#""text/plain""#, true))]
#[case::metadata(("metadata", "null", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::created_at(("created_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::created_by(("created_by", "null", r#""c""#, r#""c""#, true))]
#[case::updated_at(("updated_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::updated_by(("updated_by", "null", r#""u""#, r#""u""#, true))]
fn skill_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("skill", data(r#"{"skill_id":"s"}"#), spec);
}

#[test]
fn skill_requires_skill_id() {
    assert!(!accepts("skill", &data("{}")));
    assert!(!accepts("skill", &data(r#"{"skill_id":null}"#)));
}

#[rstest]
#[case::savings_estimated_turns(("savings_estimated_turns", "0", "2", "2", false))]
#[case::savings_estimated_actual_spend(("savings_estimated_actual_spend", "0.0", "1.5", "1.5", false))]
#[case::savings_estimated_saved_spend(("savings_estimated_saved_spend", "0.0", "2.5", "2.5", false))]
#[case::savings_estimated_baseline_models(("savings_estimated_baseline_models", "{}", r#"{"m":2}"#, r#"{"m":2}"#, false))]
fn autorouter_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("autorouter_session", autorouter_base(), spec);
}

#[rstest]
#[case::api_key("api_key")]
#[case::session_id("session_id")]
#[case::router_name("router_name")]
#[case::router_type("router_type")]
#[case::first_turn_at("first_turn_at")]
#[case::last_turn_at("last_turn_at")]
#[case::last_model("last_model")]
#[case::turns("turns")]
#[case::spend("spend")]
#[case::saved_spend("saved_spend")]
#[case::classifier_cost("classifier_cost")]
#[case::tier_turns("tier_turns")]
#[case::baseline_models("baseline_models")]
fn autorouter_required_fields(#[case] field: &str) {
    assert_required_field("autorouter_session", autorouter_base(), field);
}

#[rstest]
#[case::empty("{}", 3, 0, None)]
#[case::single(r#"{"a":5}"#, 3, 0, Some("a"))]
#[case::excluded_turns_multiple(r#"{"a":5,"b":7}"#, 3, 0, None)]
#[case::all_estimated_multiple(r#"{"a":5,"b":7}"#, 3, 3, Some("b"))]
#[case::lexical_tie(r#"{"a":7,"b":7}"#, 3, 3, Some("b"))]
fn autorouter_baseline_model(
    #[case] models: &str,
    #[case] turns: i64,
    #[case] estimated: i64,
    #[case] expected: Option<&str>,
) {
    let mut row = autorouter_base();
    set_field(&mut row, "baseline_models", data(models));
    set_field(
        &mut row,
        "turns",
        JsonData::Number(serde_json::Number::from(turns)),
    );
    set_field(
        &mut row,
        "savings_estimated_turns",
        JsonData::Number(serde_json::Number::from(estimated)),
    );
    assert_eq!(auto_router(&row).baseline_model(), expected);
}

#[rstest]
#[case::model(("model", r#""""#, r#""m""#, r#""m""#, true))]
#[case::api_base(("api_base", r#""""#, r#""/v1""#, r#""/v1""#, true))]
#[case::spend(("spend", "0.0", "1.25", "1.25", true))]
#[case::total_tokens(("total_tokens", "0", "4", "4", true))]
#[case::prompt_tokens(("prompt_tokens", "0", "4", "4", true))]
#[case::completion_tokens(("completion_tokens", "0", "4", "4", true))]
#[case::user(("user", r#""""#, r#""u""#, r#""u""#, true))]
#[case::metadata(("metadata", "{}", r#""{\"a\": 1}""#, r#"{"a":1}"#, true))]
#[case::cache_hit(("cache_hit", r#""False""#, r#""True""#, r#""True""#, true))]
#[case::cache_key(("cache_key", "null", r#""k""#, r#""k""#, true))]
#[case::request_tags(("request_tags", "null", r#""[\"x\"]""#, r#"["x"]"#, true))]
#[case::requester_ip_address(("requester_ip_address", "null", r#""1.2.3.4""#, r#""1.2.3.4""#, true))]
#[case::created_at(("created_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::updated_at(("updated_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
fn spend_log_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("spend_log", spend_log_base(), spec);
}

#[rstest]
#[case::request_id("request_id")]
#[case::api_key("api_key")]
#[case::call_type("call_type")]
fn spend_log_required_fields(#[case] field: &str) {
    assert_required_field("spend_log", spend_log_base(), field);
}

#[rstest]
#[case::request_id(("request_id", r#""00000000-0000-0000-0000-00000000002a""#, r#""explicit""#, r#""explicit""#, true))]
#[case::api_base(("api_base", r#""""#, r#""/v1""#, r#""/v1""#, true))]
#[case::model_group(("model_group", r#""""#, r#""g""#, r#""g""#, true))]
#[case::litellm_model_name(("litellm_model_name", r#""""#, r#""m""#, r#""m""#, true))]
#[case::model_id(("model_id", r#""""#, r#""id""#, r#""id""#, true))]
#[case::request_kwargs(("request_kwargs", "{}", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::exception_type(("exception_type", r#""""#, r#""e""#, r#""e""#, true))]
#[case::status_code(("status_code", r#""""#, r#""500""#, r#""500""#, true))]
#[case::exception_string(("exception_string", r#""""#, r#""s""#, r#""s""#, true))]
fn error_log_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("error_log", error_log_base(), spec);
}

#[rstest]
#[case::models(("models", "[]", r#"["m"]"#, r#"["m"]"#, true))]
#[case::spend(("spend", "0.0", "1.5", "1.5", true))]
#[case::description(("description", "null", r#""d""#, r#""d""#, true))]
#[case::model_info(("model_info", "null", r#"{"a":1}"#, r#"{"a":1}"#, true))]
#[case::budget_id(("budget_id", "null", r#""b""#, r#""b""#, true))]
#[case::litellm_budget_table(("litellm_budget_table", "null", r#"{"budget_id":"b"}"#, r#"{"budget_id":"b","soft_budget":null,"max_budget":null,"max_parallel_requests":null,"tpm_limit":null,"rpm_limit":null,"tpd_limit":null,"model_max_budget":null,"budget_duration":null,"allowed_models":null,"temp_budget_increase":null,"temp_budget_expiry":null}"#, true))]
#[case::created_at(("created_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
#[case::created_by(("created_by", "null", r#""c""#, r#""c""#, true))]
#[case::updated_at(("updated_at", "null", r#""2026-01-01T00:00:00Z""#, r#""2026-01-01T00:00:00Z""#, true))]
fn tag_fields(#[case] spec: (&str, &str, &str, &str, bool)) {
    assert_field_cases("tag", data(r#"{"tag_name":"t"}"#), spec);
}

#[test]
fn tag_requires_tag_name() {
    assert!(!accepts("tag", &data("{}")));
    assert!(!accepts("tag", &data(r#"{"tag_name":null}"#)));
}

#[test]
fn tag_null_normalization() {
    let tag = tag_table(&data(
        r#"{"tag_name":"t","models":null,"spend":null,"model_info":{"a":1}}"#,
    ));
    assert!(tag.models.is_empty());
    assert_eq!(tag.spend, Decimal::ZERO);
    assert!(tag.model_info.is_some());
    let untouched = tag_table(&data(r#"{"tag_name":"t","model_info":null}"#));
    assert!(untouched.model_info.is_none());
}

#[rstest]
#[case::healthy(r#""healthy""#, Some(r#""healthy""#))]
#[case::reachable(r#""reachable""#, Some(r#""reachable""#))]
#[case::unhealthy(r#""unhealthy""#, Some(r#""unhealthy""#))]
#[case::unknown(r#""unknown""#, Some(r#""unknown""#))]
#[case::null("null", Some("null"))]
#[case::other(r#""weird""#, None)]
fn status_enum_and_null(#[case] status: &str, #[case] expected: Option<&str>) {
    let mut row = mcp_server_base();
    set_field(&mut row, "status", data(status));
    match expected {
        Some(value) => assert_eq!(field_of(&dumped("mcp_server", &row), "status"), data(value)),
        None => assert!(!accepts("mcp_server", &row)),
    }
}

#[rstest]
#[case::client_credentials(r#""client_credentials""#, true)]
#[case::authorization_code(r#""authorization_code""#, true)]
#[case::other(r#""other""#, false)]
#[case::null("null", true)]
fn oauth_flow_literals(#[case] flow: &str, #[case] accepted: bool) {
    let mut row = mcp_server_base();
    set_field(&mut row, "oauth2_flow", data(flow));
    assert_eq!(decode_envelope("mcp_server", &row).is_ok(), accepted);
}

#[rstest]
#[case::active(r#""active""#, true)]
#[case::pending_review(r#""pending_review""#, true)]
#[case::weird(r#""weird""#, true)]
#[case::null("null", true)]
#[case::number("5", false)]
fn approval_status_free_string(#[case] status: &str, #[case] accepted: bool) {
    let mut row = mcp_server_base();
    set_field(&mut row, "approval_status", data(status));
    assert_eq!(decode_envelope("mcp_server", &row).is_ok(), accepted);
}

#[rstest]
#[case::absent(None, None)]
#[case::null(Some("null"), None)]
#[case::empty_object(Some("{}"), None)]
#[case::object(
    Some(r#"{"t":{"description":"d"}}"#),
    Some(r#"{"t":{"description":"d","input_schema":{}}}"#)
)]
#[case::text(
    Some(r#""{ \"t\": { \"description\": \"d\" } }""#),
    Some(r#"{"t":{"description":"d","input_schema":{}}}"#)
)]
fn mcp_pinned_tools(#[case] stored: Option<&str>, #[case] want: Option<&str>) {
    let mut row = mcp_server_base();
    match stored {
        Some(text) => set_field(&mut row, "pinned_tools", data(text)),
        None => remove_field(&mut row, "pinned_tools"),
    }
    let dumped = dumped("mcp_server", &row);
    let actual = field_of(&dumped, "pinned_tools");
    match want {
        Some(token) => assert_eq!(actual, data(token)),
        None => assert_eq!(actual, JsonData::Null),
    }
}

#[rstest]
#[case::array(r#"["tool"]"#)]
#[case::unknown_field(r#"{"t":{"extra":1}}"#)]
#[case::wrong_description_type(r#"{"t":{"description":false}}"#)]
fn mcp_pinned_tools_rejects_invalid_tools(#[case] invalid: &str) {
    let mut row = mcp_server_base();
    set_field(&mut row, "pinned_tools", data(invalid));
    assert!(!accepts("mcp_server", &row), "{invalid}");
}

#[rstest]
#[case::plaintext_object(r#"{"Authorization":"Bearer row-secret"}"#, "managed-row-key", true)]
#[case::json_text(
    r#""{\"Authorization\": \"Bearer row-secret\"}""#,
    "managed-row-key",
    true
)]
#[case::encrypted(r#""YRxZqp_n51DmLFJrE5ewMwpkUQt7RBVfz86OO7dO1hEoqpx1r1L8ZbJr__NBLz_I7IRDN-1hlIRQTi6LoL0y3bLHgP8qyPKS35jUEso=""#, "managed-row-key", true)]
#[case::wrong_key(r#""YRxZqp_n51DmLFJrE5ewMwpkUQt7RBVfz86OO7dO1hEoqpx1r1L8ZbJr__NBLz_I7IRDN-1hlIRQTi6LoL0y3bLHgP8qyPKS35jUEso=""#, "other-key", false)]
#[case::malformed(r#"{"Authorization":1}"#, "managed-row-key", false)]
fn mcp_secret_map(#[case] value: &str, #[case] key: &str, #[case] accepted: bool) {
    let cipher = cipher_with(key);
    let mut row = mcp_server_base();
    set_field(&mut row, "static_headers", data(value));
    let decoded = decode_with(&envelope("mcp_server", &row), &cipher);
    assert_eq!(decoded.is_ok(), accepted);
    if let Ok(FullRow::McpServer(server)) = decoded {
        assert_eq!(
            server.static_headers.unwrap().0["Authorization"].expose_secret(),
            PLAINTEXT
        );
    }
}

#[test]
fn null_env_vs_headers() {
    let absent = mcp_server(&mcp_server_base());
    assert!(absent.static_headers.is_none());
    assert!(absent.env.0.is_empty());
    let nulled = dumped(
        "mcp_server",
        &data(r#"{"server_id":"s","transport":"sse","static_headers":null,"env":null}"#),
    );
    assert_eq!(field_of(&nulled, "static_headers"), JsonData::Null);
    assert_eq!(field_of(&nulled, "env"), data("{}"));
}

#[test]
fn bad_secret_map_redacted() {
    assert_secret_map_redacted();
}

#[test]
fn binary_skill_content() {
    let skill = skill_table(&data(r#"{"skill_id":"s","file_content":"a\u0000b"}"#));
    assert_eq!(skill.file_content.unwrap().as_bytes(), [0x61, 0x00, 0x62]);
    assert!(!accepts(
        "skill",
        &data(r#"{"skill_id":"s","file_content":3}"#)
    ));
}

#[test]
fn independent_collection_defaults() {
    let mut first = mcp_server(&data(r#"{"server_id":"a","transport":"sse"}"#));
    let second = mcp_server(&data(r#"{"server_id":"b","transport":"sse"}"#));
    first.args.push("-x".to_owned());
    first.allowed_tools.push("t".to_owned());
    first.extra_headers.push("h".to_owned());
    first.byok_description.push("d".to_owned());
    first.mcp_access_groups.push("g".to_owned());
    first
        .teams
        .push(BTreeMap::from([("team_id".to_owned(), None)]));
    first.env.0.insert("A".to_owned(), SecretString::from("B"));
    assert!(second.args.is_empty());
    assert!(second.allowed_tools.is_empty());
    assert!(second.extra_headers.is_empty());
    assert!(second.byok_description.is_empty());
    assert!(second.mcp_access_groups.is_empty());
    assert!(second.teams.is_empty());
    assert!(second.env.0.is_empty());
    let mut one = tag_table(&data(r#"{"tag_name":"a"}"#));
    let two = tag_table(&data(r#"{"tag_name":"b"}"#));
    one.models.push("m".to_owned());
    assert!(two.models.is_empty());
}

#[test]
fn spend_json_default_vs_supplied() {
    let defaulted = spend_log(&spend_log_base());
    assert_eq!(defaulted.metadata, Some(JsonData::Object(BTreeMap::new())));
    assert!(defaulted.request_tags.is_none());
    let supplied = spend_log(&data(
        r#"{"request_id":"r","api_key":"k","call_type":"c","startTime":null,"endTime":null,"messages":null,"response":null,"metadata":"{\"a\": 1}","request_tags":"[\"x\"]"}"#,
    ));
    assert_eq!(
        supplied.metadata,
        Some(JsonData::Object(BTreeMap::from([(
            "a".to_owned(),
            JsonData::Number(serde_json::Number::from(1_i32))
        )])))
    );
    assert_eq!(
        supplied.request_tags,
        Some(JsonData::Array(vec![JsonData::String("x".to_owned())]))
    );
}

#[rstest]
#[case::number("3")]
#[case::boolean("true")]
#[case::array("[1]")]
#[case::object(r#"{"a":1}"#)]
fn spend_json_non_string_fields_rejected(#[case] invalid: &str) {
    let mut metadata = spend_log_base();
    set_field(&mut metadata, "metadata", data(invalid));
    assert!(!accepts("spend_log", &metadata), "{invalid}");
    let mut tags = spend_log_base();
    set_field(&mut tags, "request_tags", data(invalid));
    assert!(!accepts("spend_log", &tags), "{invalid}");
}

#[rstest]
#[case::text(r#""text""#, true)]
#[case::array("[1,2]", true)]
#[case::object(r#"{"a":1}"#, true)]
#[case::null("null", true)]
#[case::number("3", false)]
fn spend_messages_payloads(#[case] payload: &str, #[case] accepted: bool) {
    let mut row = spend_log_base();
    set_field(&mut row, "messages", data(payload));
    assert_eq!(accepts("spend_log", &row), accepted);
}

#[rstest]
#[case::spend_start_time("spend_log", "startTime")]
#[case::spend_end_time("spend_log", "endTime")]
#[case::spend_messages("spend_log", "messages")]
#[case::spend_response("spend_log", "response")]
#[case::error_start_time("error_log", "startTime")]
#[case::error_end_time("error_log", "endTime")]
fn required_nullable_fields(#[case] kind: &str, #[case] field: &str) {
    let base = if kind == "spend_log" {
        spend_log_base()
    } else {
        error_log_base()
    };
    assert_required_nullable(kind, base, field);
}

#[test]
fn shared_error_request_id_default() {
    let first = error_log(&error_log_base());
    let second = error_log(&error_log_base());
    let expected = Uuid::from_u128(INJECTED_REQUEST_ID).to_string();
    assert_eq!(first.request_id.as_deref(), Some(expected.as_str()));
    assert_eq!(second.request_id, first.request_id);
    let explicit_null = error_log(&data(
        r#"{"startTime":null,"endTime":null,"request_id":null}"#,
    ));
    assert!(explicit_null.request_id.is_none());
}

#[test]
fn explicit_error_request_id() {
    let explicit = error_log(&data(
        r#"{"startTime":null,"endTime":null,"request_id":"r-1"}"#,
    ));
    assert_eq!(explicit.request_id.as_deref(), Some("r-1"));
}

#[test]
fn managed_row_input_limits() {
    assert_input_limits();
}

#[rstest]
fn replays_the_managed_resource_rows_corpus(
    #[files("fuzz/corpus/managed_resource_rows/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode(from_utf8(&bytes).unwrap_or_default()));
}

#[tokio::test]
async fn postgres_pinned_rows() {
    let (fixture, client, tasks) = provision().await;
    assert_pinned_rows(&client).await;
    shutdown(fixture, client, tasks).await;
}
