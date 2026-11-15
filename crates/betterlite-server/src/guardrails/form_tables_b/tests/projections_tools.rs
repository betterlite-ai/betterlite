use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::project::project_model;

use super::expected::{expected, projected};

const MAX_DEPTH: usize = 100;

#[test]
fn custom_code_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "custom_code")
        .expect("custom_code has a table");
    assert!(model.ui_friendly_name.is_none());
    let fields = project_model(&model, MAX_DEPTH).expect("custom_code nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/custom_code.json"))
    );
}

#[test]
fn generic_guardrail_api_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "generic_guardrail_api")
        .expect("generic_guardrail_api has a table");
    assert_eq!(model.ui_friendly_name, Some("Generic Guardrail API"));
    let fields =
        project_model(&model, MAX_DEPTH).expect("generic_guardrail_api nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/generic_guardrail_api.json"))
    );
}

#[test]
fn tool_permission_projects_like_litellm() {
    let (_, model) = tables_b()
        .into_iter()
        .find(|(key, _)| *key == "tool_permission")
        .expect("tool_permission has a table");
    assert_eq!(
        model.ui_friendly_name,
        Some("LiteLLM Tool Permission Guardrail")
    );
    let fields = project_model(&model, MAX_DEPTH).expect("tool_permission nests below the limit");
    assert_eq!(
        projected(fields),
        expected(include_str!("../fixtures/tool_permission.json"))
    );
}
