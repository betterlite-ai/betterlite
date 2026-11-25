use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{capture_logs, row};
use crate::db_row_decrypt::model_info::model_info_for;
use crate::db_row_decrypt::warnings::Warnings;

const ECHOED_WARNING: &str = "Deployment dep-1 stores a copy of the cost map in model_info (input_cost_per_token, input_cost_per_token_above_128k_tokens); ignoring it so the deployment follows the current cost map. Set the price on litellm_params to override the cost map on purpose.";

#[test]
fn stamps_id_db_model_and_blocked_from_the_row() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"openai/gpt-4o"},
        "model_info":{"id":"stale","db_model":false,"blocked":false,"base_model":"gpt-4o"},"blocked":true}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(info.get("id"), Some(&Json::Text("dep-1".to_owned())));
    assert_eq!(info.get("db_model"), Some(&Json::Bool(true)));
    assert_eq!(info.get("blocked"), Some(&Json::Bool(true)));
    assert_eq!(info.get("member_auto_router"), Some(&Json::Bool(false)));
    assert_eq!(
        info.get("base_model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}

#[test]
fn copies_audit_fields_in_pydantic_datetime_form() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},"model_info":{},
        "created_at":"2024-05-01T12:00:00+00:00","updated_at":"2024-05-01T12:00:00.123000+02:00",
        "created_by":"admin","updated_by":"ops"}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(
        info.get("created_at"),
        Some(&Json::Text("2024-05-01T12:00:00Z".to_owned()))
    );
    assert_eq!(
        info.get("updated_at"),
        Some(&Json::Text("2024-05-01T12:00:00.123000+02:00".to_owned()))
    );
    assert_eq!(
        info.get("created_by"),
        Some(&Json::Text("admin".to_owned()))
    );
    assert_eq!(info.get("updated_by"), Some(&Json::Text("ops".to_owned())));
}

#[test]
fn formats_a_naive_timestamp_without_a_zone() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},"model_info":{},
        "created_at":"2024-05-01T12:00:00","updated_at":"2024-05-01T12:00:00.500000"}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(
        info.get("created_at"),
        Some(&Json::Text("2024-05-01T12:00:00".to_owned()))
    );
    assert_eq!(
        info.get("updated_at"),
        Some(&Json::Text("2024-05-01T12:00:00.500000".to_owned()))
    );
}

#[test]
fn drops_echoed_cost_map_pricing_with_one_warning_per_id_and_fields() {
    let (logs, _guard) = capture_logs();
    let warnings = Warnings::default();
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"key":"openai/gpt-4o","input_cost_per_token":0.5,
        "input_cost_per_token_above_128k_tokens":0.25,"base_model":"gpt-4o"}}"#,
    );
    let first = model_info_for(&row, false, &warnings);
    let second = model_info_for(&row, false, &warnings);
    assert_echoed_pricing_dropped(&first);
    assert_echoed_pricing_dropped(&second);
    assert_eq!(logs.contents().matches(ECHOED_WARNING).count(), 1);
}

#[test]
fn warns_again_for_a_different_field_set_on_the_same_deployment() {
    let (logs, _guard) = capture_logs();
    let warnings = Warnings::default();
    let first = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"key":"k","input_cost_per_token":0.5}}"#,
    );
    let second = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"key":"k","output_cost_per_token":0.5}}"#,
    );
    drop(model_info_for(&first, false, &warnings));
    drop(model_info_for(&second, false, &warnings));
    let captured = logs.contents();
    assert_eq!(
        captured
            .matches("Deployment dep-1 stores a copy of the cost map in model_info (input_cost_per_token);")
            .count(),
        1
    );
    assert_eq!(
        captured
            .matches(
                "Deployment dep-1 stores a copy of the cost map in model_info (output_cost_per_token);"
            )
            .count(),
        1
    );
}

#[test]
fn keeps_pricing_when_the_cost_map_key_is_absent() {
    let (logs, _guard) = capture_logs();
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"input_cost_per_token":0.5,"base_model":"gpt-4o"}}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(
        info.get("input_cost_per_token").map(json_text),
        Some("0.5".to_owned())
    );
    assert!(!logs.contents().contains("stores a copy of the cost map"));
}

fn json_text(value: &Json) -> String {
    serde_json::to_string(value).unwrap()
}

fn assert_echoed_pricing_dropped(info: &JsonObject) {
    assert_eq!(info.get("input_cost_per_token"), None);
    assert_eq!(info.get("input_cost_per_token_above_128k_tokens"), None);
    assert_eq!(
        info.get("base_model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
    assert_eq!(
        info.get("key"),
        Some(&Json::Text("openai/gpt-4o".to_owned()))
    );
}

#[test]
fn keeps_the_newest_cost_map_copies_after_the_cache_fills() {
    let warnings = Warnings::default();
    (0..4096_usize).for_each(|index| {
        let fields = [format!("field_{index}")];
        assert!(warnings.first_cost_map_copy("dep-1", &fields));
    });
    assert!(!warnings.first_cost_map_copy("dep-1", &["field_0".to_owned()]));
}

#[test]
fn a_row_without_model_info_gets_the_minimal_info() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},"model_info":null}"#,
    );
    let hidden = model_info_for(&row, false, &Warnings::default());
    assert_eq!(hidden.get("id"), Some(&Json::Text("dep-1".to_owned())));
    assert_eq!(hidden.get("db_model"), Some(&Json::Bool(false)));
    assert_eq!(hidden.get("member_auto_router"), Some(&Json::Bool(false)));
    let database = model_info_for(&row, true, &Warnings::default());
    assert_eq!(database.get("db_model"), Some(&Json::Bool(true)));
}

#[test]
fn the_row_audit_columns_overwrite_what_model_info_stored() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"created_by":"stored","updated_by":"stored","created_at":"2024-01-01T00:00:00Z"},
        "created_by":"admin"}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(
        info.get("created_by"),
        Some(&Json::Text("admin".to_owned()))
    );
    assert_eq!(info.get("updated_by"), None);
    assert_eq!(info.get("created_at"), None);
}

#[test]
fn null_model_info_values_are_dropped() {
    let row = row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"},
        "model_info":{"base_model":null,"note":null,"member_auto_router":null}}"#,
    );
    let info = model_info_for(&row, false, &Warnings::default());
    assert_eq!(info.get("base_model"), None);
    assert_eq!(info.get("note"), None);
    assert_eq!(info.get("member_auto_router"), Some(&Json::Bool(false)));
}
