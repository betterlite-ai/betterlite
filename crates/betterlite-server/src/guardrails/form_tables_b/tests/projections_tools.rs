use serde_json::json;

use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

const MAX_DEPTH: usize = 100;

#[test]
fn custom_code_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "custom_code")
        .expect("custom_code has a table");
    let mut fields = project_model(&model, MAX_DEPTH).expect("custom_code nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/custom_code.rs"));
}

#[test]
fn generic_guardrail_api_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "generic_guardrail_api")
        .expect("generic_guardrail_api has a table");
    let mut fields =
        project_model(&model, MAX_DEPTH).expect("generic_guardrail_api nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(
        json!(fields),
        include!("../fixtures/generic_guardrail_api.rs")
    );
}

#[test]
fn tool_permission_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "tool_permission")
        .expect("tool_permission has a table");
    let mut fields =
        project_model(&model, MAX_DEPTH).expect("tool_permission nests below the limit");
    fields.insert(
        "ui_friendly_name".to_owned(),
        model
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(fields), include!("../fixtures/tool_permission.rs"));
}
