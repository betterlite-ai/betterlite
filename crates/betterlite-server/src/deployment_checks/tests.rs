#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;

use crate::deployment_checks::{
    check_access_windows, check_capability_limits, check_complexity_placement,
    check_max_agentic_loops, pin_complexity_model_id,
};

const COMPLEXITY_DEPLOYMENT: &str = r#"{
    "model_name": "pinned",
    "litellm_params": {
        "model": "auto_router/complexity_router",
        "complexity_router_config": {"classifier_type": "heuristic"}
    }
}"#;

// litellm's Router.generate_model_id over COMPLEXITY_DEPLOYMENT, as the pinned LiteLLM prints it
const COMPLEXITY_ID: &str = "20298a2ef5c89d7fcc13f81d7484ef84e58fbcbe943793025fb7cb2103a6fbb0";

fn entry(source: &str) -> JsonObject {
    serde_json::from_str(source).expect("the deployment fixture is a JSON object")
}

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn model_info(model: &JsonObject) -> &JsonObject {
    match model.get("model_info") {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => {
            panic!("the fixture carries a model_info object")
        }
    }
}

#[rstest]
#[case::zero(
    r#"{"model_name": "zero-loops", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": 0}}"#,
    "litellm_params.max_agentic_loops on model 'zero-loops' must be at least 1, got 0"
)]
#[case::negative(
    r#"{"model_name": "negative", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": -2}}"#,
    "litellm_params.max_agentic_loops on model 'negative' must be at least 1, got -2"
)]
#[case::boolean(
    r#"{"model_name": "flag", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": true}}"#,
    "litellm_params.max_agentic_loops on model 'flag' must be an integer, got True"
)]
#[case::fraction(
    r#"{"model_name": "fraction", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": 1.5}}"#,
    "litellm_params.max_agentic_loops on model 'fraction' must be an integer, got 1.5"
)]
#[case::text(
    r#"{"model_name": "word", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": "many"}}"#,
    "litellm_params.max_agentic_loops on model 'word' must be an integer, got 'many'"
)]
#[case::list(
    r#"{"model_name": "list", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": [2]}}"#,
    "litellm_params.max_agentic_loops on model 'list' must be an integer, got [2]"
)]
#[case::object(
    r#"{"model_name": "dict", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": {"limit": 2}}}"#,
    "litellm_params.max_agentic_loops on model 'dict' must be an integer, got {'limit': 2}"
)]
#[case::huge_negative(
    r#"{"model_name": "huge", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": -9223372036854775807}}"#,
    "litellm_params.max_agentic_loops on model 'huge' must be at least 1, got -9223372036854775807"
)]
fn refuses_a_ceiling_the_agentic_loop_cannot_honor(#[case] fixture: &str, #[case] expected: &str) {
    let error = check_max_agentic_loops(&entry(fixture)).expect_err("the ceiling is refused");
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::absent(r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o"}}"#)]
#[case::null(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": null}}"#
)]
#[case::whole(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": 3}}"#
)]
#[case::whole_float(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": 3.0}}"#
)]
#[case::numeric_text(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": "5"}}"#
)]
#[case::largest_ceiling(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "max_agentic_loops": 4294967295}}"#
)]
#[case::non_mapping_params(r#"{"model_name": "plain", "litellm_params": "unset"}"#)]
fn accepts_a_ceiling_the_agentic_loop_can_honor(#[case] fixture: &str) {
    check_max_agentic_loops(&entry(fixture)).unwrap();
}

const MISPLACED_MESSAGE: &str = "model 'complexity': litellm_params sets complexity_router_config settings directly: tiers. The router reads these only from complexity_router_config, so there they configure nothing and are forwarded to the provider as unknown request params, which rejects the call. Move them under complexity_router_config.";

const SORTED_MISPLACED_MESSAGE: &str = "model 'sorted': litellm_params sets complexity_router_config settings directly: classifier_type, tiers. The router reads these only from complexity_router_config, so there they configure nothing and are forwarded to the provider as unknown request params, which rejects the call. Move them under complexity_router_config.";

#[rstest]
#[case::complexity_router(
    r#"{"model_name": "complexity", "litellm_params": {"model": "auto_router/complexity_router", "tiers": ["fast"]}}"#,
    MISPLACED_MESSAGE
)]
#[case::named_complexity_router(
    r#"{"model_name": "sorted", "litellm_params": {"model": "auto_router/complexity_router/fast", "tiers": ["fast"], "classifier_type": "heuristic"}}"#,
    SORTED_MISPLACED_MESSAGE
)]
#[case::config_carrier(
    r#"{"model_name": "sorted", "litellm_params": {"model": "openai/gpt-4o", "complexity_router_config": {}, "tiers": ["fast"], "classifier_type": "heuristic"}}"#,
    SORTED_MISPLACED_MESSAGE
)]
#[case::null_value_is_still_written(
    r#"{"model_name": "nested", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": {}, "tiers": null}}"#,
    "model 'nested': litellm_params sets complexity_router_config settings directly: tiers. The router reads these only from complexity_router_config, so there they configure nothing and are forwarded to the provider as unknown request params, which rejects the call. Move them under complexity_router_config."
)]
#[case::strategy_field_without_router(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "complexity_router_default_model": "gpt-4o-mini", "tiers": ["fast"]}}"#,
    "model 'plain': litellm_params sets complexity_router_config settings directly: tiers. The router reads these only from complexity_router_config, so there they configure nothing and are forwarded to the provider as unknown request params, which rejects the call. Move them under complexity_router_config."
)]
fn refuses_a_complexity_setting_written_above_the_router_config(
    #[case] fixture: &str,
    #[case] expected: &str,
) {
    let error = check_complexity_placement(&entry(fixture)).expect_err("the placement is refused");
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::not_a_complexity_router(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "tiers": ["fast"]}}"#
)]
#[case::router_with_nested_settings(
    r#"{"model_name": "nested", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": {"tiers": ["fast"]}}}"#
)]
#[case::non_mapping_params(r#"{"model_name": "plain", "litellm_params": ["model"]}"#)]
#[case::null_strategy_field_does_not_claim(
    r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "complexity_router_default_model": null, "tiers": ["fast"]}}"#
)]
fn accepts_a_deployment_with_its_complexity_settings_placed(#[case] fixture: &str) {
    check_complexity_placement(&entry(fixture)).unwrap();
}

const BAD_START: &str = r#"{"model_name": "windows", "model_info": {"access_windows": [{"start": "9:00", "end": "17:00", "timezone": "UTC", "team_ids": ["t1"]}]}}"#;

const NOT_A_LIST: &str = r#"{"model_name": "windows", "model_info": {"access_windows": "nine"}}"#;

#[rstest]
#[case::short_time(
    BAD_START,
    "model 'windows': invalid model_info.access_windows: 0.start: must be a time like 09:00"
)]
#[case::not_a_list(
    NOT_A_LIST,
    "model 'windows': invalid model_info.access_windows: : access_windows must be a list of windows"
)]
fn refuses_a_window_the_deployment_would_drop(#[case] fixture: &str, #[case] expected: &str) {
    let error = check_access_windows(&entry(fixture), &database())
        .expect_err("the access window is refused");
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::valid_window(
    r#"{"model_name": "windows", "model_info": {"access_windows": [{"start": "09:00", "end": "17:00", "timezone": "UTC", "team_ids": ["t1"]}]}}"#
)]
#[case::no_windows(r#"{"model_name": "windows", "model_info": {}}"#)]
#[case::no_model_info(r#"{"model_name": "windows"}"#)]
#[case::non_mapping_model_info(r#"{"model_name": "windows", "model_info": ["access_windows"]}"#)]
fn accepts_a_deployment_whose_windows_parse(#[case] fixture: &str) {
    check_access_windows(&entry(fixture), &database()).unwrap();
}

const HEURISTIC_LIMIT_MESSAGE: &str = "config.yaml model_list: At most 1 auto-router(s) with classifier_type 'heuristic_v2' can be registered but this would make 2. Use classifier_type 'heuristic' for this router or remove an existing heuristic_v2 router. A LiteLLM license with the 'auto_router' feature lifts the limit.";

const CUSTOMIZATION_LIMIT_MESSAGE: &str = "config.yaml model_list: At most 0 auto-router(s) with operator-defined tier_definitions or an operator-written classifier prompt can be registered but this would make 1. Use the shipped tiers and classifier prompt for this router or remove an existing router with tier_definitions or its own classifier prompt. A LiteLLM license with the 'auto_router' feature lifts the limit.";

const CAPABILITY_LIMIT_MESSAGE: &str = "config.yaml model_list: At most 0 auto-router(s) with classifier_type 'capability' (Capability) can be registered but this would make 1. Use a different classifier or remove an existing Capability router. A LiteLLM license with the 'auto_router' feature lifts the limit.";

const LLM_V2_LIMIT_MESSAGE: &str = "config.yaml model_list: At most 1 auto-router(s) with classifier_type 'llm_v2' (Fuse v2) can be registered but this would make 2. Use a different classifier or remove an existing Fuse v2 router. A LiteLLM license with the 'auto_router' feature lifts the limit.";

fn heuristic_v2(name: &str) -> JsonObject {
    entry(&format!(
        r#"{{"model_name": "{name}", "litellm_params": {{"model": "auto_router/complexity_router", "complexity_router_config": {{"classifier_type": "heuristic_v2"}}}}}}"#
    ))
}

fn with_classifier(name: &str, classifier: &str) -> JsonObject {
    entry(&format!(
        r#"{{"model_name": "{name}", "litellm_params": {{"model": "auto_router/complexity_router", "complexity_router_config": {{"classifier_type": "{classifier}"}}}}}}"#
    ))
}

#[rstest]
#[case::heuristic_v2(
    vec![heuristic_v2("a"), heuristic_v2("b")],
    1_usize,
    HEURISTIC_LIMIT_MESSAGE
)]
#[case::llm_v2(
    vec![with_classifier("a", "llm_v2"), with_classifier("b", "llm_v2")],
    1_usize,
    LLM_V2_LIMIT_MESSAGE
)]
#[case::customization(
    vec![entry(
        r#"{"model_name": "custom", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": {"classifier_type": "heuristic", "tier_definitions": [{"name": "fast"}]}}}"#
    )],
    0_usize,
    CUSTOMIZATION_LIMIT_MESSAGE
)]
#[case::capability_claims_one_capability(
    vec![entry(
        r#"{"model_name": "cap", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": {"classifier_type": "capability", "tier_definitions": [{"name": "fast"}]}}}"#
    )],
    0_usize,
    CAPABILITY_LIMIT_MESSAGE
)]
fn refuses_more_gated_routers_than_the_license_allows(
    #[case] model_list: Vec<JsonObject>,
    #[case] limit: usize,
    #[case] expected: &str,
) {
    let error = check_capability_limits(&model_list, Some(limit))
        .expect_err("the capability count is over the limit");
    assert_eq!(error.to_string(), expected);
}

#[rstest]
#[case::unlimited(vec![heuristic_v2("a"), heuristic_v2("b")], None)]
#[case::at_the_limit(vec![heuristic_v2("a"), heuristic_v2("b")], Some(2))]
#[case::under_the_limit(vec![heuristic_v2("a")], Some(2))]
#[case::empty(vec![], Some(0))]
#[case::non_router_with_the_classifier(
    vec![entry(
        r#"{"model_name": "plain", "litellm_params": {"model": "openai/gpt-4o", "complexity_router_config": {"classifier_type": "heuristic_v2"}}}"#
    )],
    Some(0)
)]
#[case::non_mapping_params(
    vec![entry(r#"{"model_name": "plain", "litellm_params": "unset"}"#)],
    Some(0)
)]
#[case::customization_fits(
    vec![entry(
        r#"{"model_name": "custom", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": {"classifier_type": "heuristic", "tier_definitions": [{"name": "fast"}]}}}"#
    )],
    Some(1)
)]
fn accepts_a_model_list_inside_the_license_limit(
    #[case] model_list: Vec<JsonObject>,
    #[case] limit: Option<usize>,
) {
    check_capability_limits(&model_list, limit).unwrap();
}

fn assert_pinned_id(model: &JsonObject) {
    assert_eq!(
        model_info(model).get("id"),
        Some(&Json::Text(COMPLEXITY_ID.to_owned()))
    );
}

#[test]
fn pins_the_id_when_model_info_is_missing() {
    let mut model = entry(COMPLEXITY_DEPLOYMENT);
    pin_complexity_model_id(&mut model);
    assert_pinned_id(&model);
}

#[test]
fn pins_the_id_when_model_info_has_none() {
    let mut model = entry(
        r#"{
            "model_name": "pinned",
            "litellm_params": {
                "model": "auto_router/complexity_router",
                "complexity_router_config": {"classifier_type": "heuristic"}
            },
            "model_info": {"db_model": false}
        }"#,
    );
    pin_complexity_model_id(&mut model);
    assert_pinned_id(&model);
    assert_eq!(model_info(&model).get("db_model"), Some(&Json::Bool(false)));
}

#[test]
fn pins_the_id_when_model_info_id_is_null() {
    let mut model = entry(
        r#"{
            "model_name": "pinned",
            "litellm_params": {
                "model": "auto_router/complexity_router",
                "complexity_router_config": {"classifier_type": "heuristic"}
            },
            "model_info": {"id": null}
        }"#,
    );
    pin_complexity_model_id(&mut model);
    assert_pinned_id(&model);
}

#[test]
fn replaces_a_model_info_that_is_not_a_mapping() {
    let mut model = entry(
        r#"{
            "model_name": "pinned",
            "litellm_params": {
                "model": "auto_router/complexity_router",
                "complexity_router_config": {"classifier_type": "heuristic"}
            },
            "model_info": ["loose"]
        }"#,
    );
    pin_complexity_model_id(&mut model);
    assert_pinned_id(&model);
    assert_eq!(model_info(&model).len(), 1_usize);
}

#[rstest]
#[case::declared_text_id(
    r#"{
        "model_name": "pinned",
        "litellm_params": {
            "model": "auto_router/complexity_router",
            "complexity_router_config": {"classifier_type": "heuristic"}
        },
        "model_info": {"id": "chosen"}
    }"#
)]
#[case::declared_number_id(
    r#"{
        "model_name": "pinned",
        "litellm_params": {
            "model": "auto_router/complexity_router",
            "complexity_router_config": {"classifier_type": "heuristic"}
        },
        "model_info": {"id": 7}
    }"#
)]
#[case::no_complexity_config(
    r#"{"model_name": "pinned", "litellm_params": {"model": "openai/gpt-4o"}}"#
)]
#[case::complexity_config_not_a_mapping(
    r#"{"model_name": "pinned", "litellm_params": {"model": "auto_router/complexity_router", "complexity_router_config": "loose"}}"#
)]
#[case::non_mapping_params(r#"{"model_name": "pinned", "litellm_params": "loose"}"#)]
fn leaves_a_deployment_without_an_unset_complexity_id_alone(#[case] fixture: &str) {
    let mut model = entry(fixture);
    let before = model.clone();
    pin_complexity_model_id(&mut model);
    assert_eq!(model, before);
}
