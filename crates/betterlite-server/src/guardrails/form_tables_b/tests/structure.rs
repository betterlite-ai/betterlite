use rstest::rstest;
use serde_json::json;

use crate::guardrails::form_tables_b::tables_b;
use crate::guardrails::provider_forms::core_tables::core_tables;
use crate::guardrails::provider_forms::project::project_model;
use crate::guardrails::provider_forms::spec::{FormField, FormModel, FormShape};

const MAX_DEPTH: usize = 100;

fn table(provider: &str) -> FormModel {
    tables_b()
        .into_iter()
        .find(|(key, _)| *key == provider)
        .expect("every table key is in tables_b")
        .1
}

fn field<'model>(model: &'model FormModel, name: &str) -> &'model FormField {
    model
        .fields
        .iter()
        .find(|candidate| candidate.name == name)
        .expect("the table holds the field")
}

fn nested<'model>(model: &'model FormModel, name: &str) -> &'model FormModel {
    match &field(model, name).shape {
        FormShape::Nested(inner) => inner.as_ref(),
        other => panic!("{name} is a nested model, not {other:?}"),
    }
}

#[test]
fn tables_b_holds_every_registry_key_in_order() {
    let keys: Vec<&str> = tables_b().iter().map(|(key, _)| *key).collect();
    assert_eq!(
        keys,
        [
            "agent_365",
            "deepkeep",
            "vigil_guard",
            "cato_networks",
            "custom_code",
            "repelloai",
            "aporia",
            "tool_permission",
            "ibm_guardrails",
            "pangea",
            "prompt_security",
            "enkryptai",
            "straiker",
            "cisco_ai_defense",
            "aim",
            "singulr",
            "onyx",
            "akto",
            "pillar",
            "lasso",
            "generic_guardrail_api",
            "ovalix",
            "compresr",
            "model_armor",
        ]
    );
}

#[test]
fn only_custom_code_has_no_friendly_name() {
    let unnamed: Vec<&str> = tables_b()
        .into_iter()
        .filter(|(_, model)| model.ui_friendly_name.is_none())
        .map(|(key, _)| key)
        .collect();
    assert_eq!(unnamed, ["custom_code"]);
}

#[test]
fn custom_code_falls_back_to_its_field_name() {
    let model = table("custom_code");
    assert!(model.ui_friendly_name.is_none());
    let code = field(&model, "custom_code");
    assert!(code.description.is_none());
    assert!(code.required);
    assert_eq!(code.default, Some(json!(null)));
    assert!(matches!(&code.shape, FormShape::Text));
}

#[rstest]
#[case::aim("aim")]
#[case::cato_networks("cato_networks")]
fn inspect_embeddings_projects_as_bool(#[case] provider: &str) {
    let model = table(provider);
    let inspect_embeddings = field(&model, "inspect_embeddings");
    assert_eq!(inspect_embeddings.extra.ui_type.as_deref(), Some("bool"));
    let fields = project_model(&model, MAX_DEPTH).expect("the table nests below the limit");
    assert_eq!(fields["inspect_embeddings"]["type"], json!("bool"));
}

#[test]
fn nested_optional_params_hold_the_child_model() {
    let model = table("compresr");
    let optional_params = nested(&model, "optional_params");
    assert!(optional_params.ui_friendly_name.is_none());
    assert_eq!(optional_params.fields.len(), 14);
    let ratio = field(optional_params, "target_compression_ratio");
    assert!(matches!(&ratio.shape, FormShape::Number));
    let compression_params = field(optional_params, "compression_params");
    assert!(matches!(
        &compression_params.shape,
        FormShape::Mapping { .. }
    ));
}

#[test]
fn generic_guardrail_api_nests_its_optional_params_class() {
    let model = table("generic_guardrail_api");
    let optional_params = nested(&model, "optional_params");
    assert_eq!(optional_params.fields.len(), 6);
    let transform_mode = field(optional_params, "streaming_transform_mode");
    assert!(matches!(
        &transform_mode.shape,
        FormShape::Choice(options) if *options == vec![json!("block_only"), json!("incremental_diff")]
    ));
}

#[test]
fn repelloai_keeps_optional_params_a_plain_string() {
    let model = table("repelloai");
    let optional_params = field(&model, "optional_params");
    assert!(matches!(&optional_params.shape, FormShape::Text));
    assert!(optional_params.default.is_none());
}

#[test]
fn enkryptai_takes_fields_from_both_parents() {
    let model = table("enkryptai");
    let names: Vec<&str> = model
        .fields
        .iter()
        .map(|candidate| candidate.name)
        .collect();
    assert_eq!(
        names,
        [
            "api_base",
            "api_key",
            "block_on_violation",
            "deployment_name",
            "detectors",
            "policy_name",
        ]
    );
    let detectors = field(&model, "detectors");
    assert!(matches!(&detectors.shape, FormShape::RawObject));
    let block_on_violation = field(&model, "block_on_violation");
    assert_eq!(block_on_violation.default, Some(json!(true)));
}

#[test]
fn straiker_secret_flag_adds_nothing_to_the_output() {
    let model = table("straiker");
    let api_key = field(&model, "api_key");
    assert!(api_key.required);
    assert_eq!(api_key.default, Some(json!(null)));
    assert!(api_key.extra.ui_type.is_none());
    assert!(api_key.extra.type_override.is_none());
    assert!(api_key.extra.options.is_none());
    assert!(api_key.extra.min.is_none());
    assert!(api_key.extra.max.is_none());
    assert!(api_key.extra.step.is_none());
    assert!(api_key.extra.default_value.is_none());
    let fields = project_model(&model, MAX_DEPTH).expect("straiker nests below the limit");
    assert_eq!(
        fields["api_key"],
        json!({
            "description": "Straiker DefendAI environment API key (Bearer token). Env: STRAIKER_API_KEY.",
            "required": true,
            "type": "string",
            "default_value": null,
        })
    );
}

#[test]
fn ibm_detector_params_and_extra_headers_default_to_null() {
    let model = table("ibm_guardrails");
    let optional_params = nested(&model, "optional_params");
    let detector_params = field(optional_params, "detector_params");
    assert_eq!(detector_params.default, Some(json!(null)));
    assert!(matches!(&detector_params.shape, FormShape::Mapping { .. }));
    let extra_headers = field(optional_params, "extra_headers");
    assert_eq!(extra_headers.default, Some(json!(null)));
}

#[test]
fn deepkeep_lasso_and_pillar_use_the_hook_models() {
    let deepkeep = table("deepkeep");
    assert!(
        deepkeep
            .fields
            .iter()
            .any(|candidate| candidate.name == "deepkeep_firewall_id")
    );
    let deepkeep_params = nested(&deepkeep, "optional_params");
    let fallback = field(deepkeep_params, "unreachable_fallback");
    assert!(matches!(&fallback.shape, FormShape::Text));
    assert_eq!(fallback.default, Some(json!("fail_closed")));
    let lasso = table("lasso");
    let lasso_params = nested(&lasso, "optional_params");
    assert!(
        lasso_params
            .fields
            .iter()
            .any(|candidate| candidate.name == "conversation_id")
    );
    assert!(
        !lasso_params
            .fields
            .iter()
            .any(|candidate| candidate.name == "mask")
    );
    let pillar = table("pillar");
    let pillar_params = nested(&pillar, "optional_params");
    let flagged = field(pillar_params, "on_flagged_action");
    assert_eq!(flagged.default, Some(json!("monitor")));
}

#[test]
fn tool_permission_matches_the_fixed_route_entry() {
    let (_, core) = core_tables()
        .into_iter()
        .find(|(key, _)| *key == "tool_permission")
        .expect("the core tables hold the fixed tool_permission entry");
    let registry = table("tool_permission");
    let mut core_fields =
        project_model(&core, MAX_DEPTH).expect("the core entry nests below the limit");
    core_fields.insert(
        "ui_friendly_name".to_owned(),
        core.ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    let mut registry_fields =
        project_model(&registry, MAX_DEPTH).expect("the registry entry nests below the limit");
    registry_fields.insert(
        "ui_friendly_name".to_owned(),
        registry
            .ui_friendly_name
            .map_or(json!(null), |name| json!(name)),
    );
    assert_eq!(json!(core_fields), json!(registry_fields));
}
