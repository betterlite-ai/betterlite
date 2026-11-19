use rstest::rstest;

use crate::router_dependencies::config_write::validate_complexity_router_config_write;
use crate::router_dependencies::error::{ComplexityConfigWriteError, StrategyRouterWriteError};
use crate::router_dependencies::validation::{
    validate_complexity_router_config_placement, validate_strategy_router_model_write,
};
use crate::router_dependencies::{AUTO_ROUTER_MODEL_PREFIX, STRATEGY_ROUTER_PARAM_FIELDS};

use super::{json, names, params};

fn rejected_location(config: &str) -> String {
    match validate_complexity_router_config_write(Some(&json(config))) {
        Err(
            ComplexityConfigWriteError::Invalid { location, .. }
            | ComplexityConfigWriteError::InvalidField { location, .. },
        ) => location,
        other => panic!("expected a rejected config, got {other:?}"),
    }
}

#[test]
fn the_router_prefix_is_exactly_litellms() {
    assert_eq!(AUTO_ROUTER_MODEL_PREFIX, "auto_router/");
}

#[test]
fn the_router_param_fields_are_exactly_litellms_set() {
    assert_eq!(
        STRATEGY_ROUTER_PARAM_FIELDS,
        [
            "auto_router_config",
            "auto_router_config_path",
            "auto_router_default_model",
            "auto_router_embedding_model",
            "auto_router_max_input_chars",
            "complexity_router_config",
            "complexity_router_default_model",
            "adaptive_router_config",
            "quality_router_config",
            "quality_router_default_model",
        ]
    );
}

#[test]
fn every_router_param_field_is_recognized_on_a_regular_model() {
    for field in STRATEGY_ROUTER_PARAM_FIELDS {
        let error = validate_strategy_router_model_write("gpt-4o", &names(&[field]))
            .expect_err("a regular model carrying a router field is rejected");
        assert!(
            error.to_string().contains(&format!("({field})")),
            "{field}: {error}"
        );
    }
    assert!(validate_strategy_router_model_write("gpt-4o", &names(&["embedding_model"])).is_ok());
}

#[test]
fn a_regular_model_carrying_router_settings_reads_exactly() {
    let error = validate_strategy_router_model_write(
        "gpt-4o",
        &names(&["auto_router_config", "adaptive_router_config", "model"]),
    )
    .expect_err("a regular model carrying a router field is rejected");
    assert_eq!(
        error.to_string(),
        "litellm_params.model='gpt-4o' does not start with 'auto_router/' but the deployment \
         carries auto-router settings (adaptive_router_config, auto_router_config), so the router \
         could not load it. Keep the 'auto_router/' prefix; to change the name clients call, edit \
         the public model_name instead."
    );
}

#[test]
fn a_repeated_prefix_names_the_remainder() {
    let error = validate_strategy_router_model_write("auto_router/auto_router/x", &names(&[]))
        .expect_err("a repeated prefix is rejected");
    assert_eq!(
        error.to_string(),
        "litellm_params.model='auto_router/auto_router/x' repeats the 'auto_router/' prefix, so the \
         router could not load it. Use 'auto_router/x'; to change the name clients call, edit the \
         public model_name instead."
    );
}

#[test]
fn an_empty_router_name_is_rejected_even_with_its_settings() {
    let error =
        validate_strategy_router_model_write("auto_router/", &names(&["auto_router_config"]))
            .expect_err("an empty router name is rejected");
    assert_eq!(
        error.to_string(),
        "litellm_params.model='auto_router/' is missing the router name after the 'auto_router/' prefix."
    );
}

#[rstest]
#[case::semantic(
    "auto_router/support-bot",
    vec![],
    "auto_router_config or auto_router_config_path; auto_router_default_model; auto_router_embedding_model"
)]
#[case::semantic_one_group(
    "auto_router/support-bot",
    vec!["auto_router_config_path", "auto_router_embedding_model"],
    "auto_router_default_model"
)]
#[case::complexity(
    "auto_router/complexity_router",
    vec![],
    "complexity_router_config or complexity_router_default_model"
)]
#[case::adaptive("auto_router/adaptive_router", vec![], "adaptive_router_config")]
#[case::quality(
    "auto_router/quality_router",
    vec![],
    "quality_router_config or quality_router_default_model"
)]
fn a_write_missing_settings_names_every_group(
    #[case] model: &str,
    #[case] present: Vec<&str>,
    #[case] expected: &str,
) {
    let error = validate_strategy_router_model_write(model, &names(&present))
        .expect_err("the write is missing settings");
    match error {
        StrategyRouterWriteError::MissingSettings { requirements, .. } => {
            assert_eq!(requirements, expected);
        }
        other => panic!("expected missing settings, got {other:?}"),
    }
}

#[test]
fn a_missing_settings_message_names_the_router_kind() {
    let error = validate_strategy_router_model_write("auto_router/support-bot", &names(&[]))
        .expect_err("the write is missing settings");
    assert_eq!(
        error.to_string(),
        "litellm_params.model='auto_router/support-bot' selects the semantic router, which \
         requires auto_router_config or auto_router_config_path; auto_router_default_model; \
         auto_router_embedding_model in litellm_params."
    );
}

#[test]
fn a_complexity_model_carrying_only_quality_fields_is_rejected() {
    let error = validate_strategy_router_model_write(
        "auto_router/complexity_router",
        &names(&["quality_router_config"]),
    )
    .expect_err("the settings belong to another router kind");
    assert!(matches!(
        &error,
        StrategyRouterWriteError::MissingSettings { .. }
    ));
    assert!(error.to_string().contains("selects the complexity router"));
}

#[test]
fn a_placement_violation_lists_every_key_in_sorted_order() {
    let litellm_params = params(
        r#"{"model":"gpt-4o","tier_labels":{},"adaptive":false,"match_threshold":0.5,
            "complexity_router_config":{}}"#,
    );
    let error = validate_complexity_router_config_placement(Some(&litellm_params))
        .expect_err("the settings sit one level too high");
    assert_eq!(
        error.to_string(),
        "litellm_params sets complexity_router_config settings directly: adaptive, \
         match_threshold, tier_labels. The router reads these only from complexity_router_config, \
         so there they configure nothing and are forwarded to the provider as unknown request \
         params, which rejects the call. Move them under complexity_router_config."
    );
}

#[test]
fn a_clean_placement_is_accepted() {
    let unrelated =
        params(r#"{"model":"gpt-4o","complexity_router_config":{"tiers":{}},"api_key":"x"}"#);
    assert!(validate_complexity_router_config_placement(Some(&unrelated)).is_ok());
    assert!(validate_complexity_router_config_placement(Some(&params("{}"))).is_ok());
}

#[rstest]
#[case::plan_mode(r#"{"plan_mode_min_tier":"NOPE"}"#, "complexity_router_config")]
#[case::heuristic_first_unused(
    r#"{"heuristic_first_max_tier":"MEDIUM"}"#,
    "complexity_router_config"
)]
#[case::heuristic_first_required(
    r#"{"classifier_type":"heuristic_first"}"#,
    "complexity_router_config"
)]
#[case::hybrid_without_margin(r#"{"classifier_type":"hybrid"}"#, "complexity_router_config")]
#[case::hybrid_margin_unused(r#"{"hybrid_boundary_margin":0.5}"#, "complexity_router_config")]
#[case::hybrid_margin_out_of_range(
    r#"{"classifier_type":"hybrid","hybrid_boundary_margin":1.5}"#,
    "hybrid_boundary_margin"
)]
#[case::hybrid_margin_negative(
    r#"{"classifier_type":"hybrid","hybrid_boundary_margin":-0.1}"#,
    "hybrid_boundary_margin"
)]
#[case::blank_prompt(
    r#"{"classifier_type":"llm","classification_prompt":"  "}"#,
    "classification_prompt"
)]
#[case::blank_fallback(r#"{"fallback_tier":"  "}"#, "fallback_tier")]
#[case::semantic_without_embedding(
    r#"{"semantic_keyword_matching":true}"#,
    "complexity_router_config"
)]
#[case::semantic_without_rules(
    r#"{"semantic_keyword_matching":true,"embedding_model":"embed-x"}"#,
    "complexity_router_config"
)]
#[case::keyword_rule_unknown_tier(
    r#"{"keyword_tier_rules":[{"tier":"NOPE","keywords":["k"]}]}"#,
    "complexity_router_config"
)]
#[case::stall_with_session(
    r#"{"stall_escalation_enabled":true,"session_affinity":true}"#,
    "complexity_router_config"
)]
#[case::stall_with_user_turn(
    r#"{"stall_escalation_enabled":true,"classification_mode":"user_turn"}"#,
    "complexity_router_config"
)]
#[case::stall_threshold_above_window(
    r#"{"stall_escalation_enabled":true,"stall_escalation_window":2,
        "stall_escalation_repeat_threshold":5}"#,
    "complexity_router_config"
)]
#[case::plugins_with_adaptive(
    r#"{"plugins":["pkg.Plugin"],"adaptive":true}"#,
    "complexity_router_config"
)]
#[case::non_reasoning_tier_under_a_heuristic(
    r#"{"enable_non_reasoning_tier":true}"#,
    "complexity_router_config"
)]
#[case::custom_tiers_with_adaptive(
    r#"{"classifier_type":"llm","fallback_tier":"ECO",
        "tier_definitions":[{"name":"ECO","description":"cheap"},
                            {"name":"DEEP","description":"hard"}],
        "tiers":{"ECO":"eco","DEEP":"deep"},"adaptive":true}"#,
    "complexity_router_config"
)]
#[case::dimension_rule(
    r#"{"custom_dimensions":[{"name":"","weight":0.5,"keywords":["k"]}]}"#,
    "complexity_router_config"
)]
fn a_config_write_names_the_failing_location(#[case] config: &str, #[case] expected: &str) {
    assert_eq!(rejected_location(config), expected);
}

#[rstest]
#[case::classifier_type(r#"{"classifier_type":"nope"}"#, "classifier_type")]
#[case::tiers_type(r#"{"tiers":5}"#, "tiers")]
#[case::tier_pool_type(r#"{"tiers":{"SIMPLE":5}}"#, "tiers.SIMPLE")]
#[case::definitions_type(r#"{"tier_definitions":{}}"#, "tier_definitions")]
#[case::definition_element(r#"{"tier_definitions":[{"description":"x"}]}"#, "tier_definitions.0")]
#[case::dimension_weight(
    r#"{"custom_dimensions":[{"name":"d","weight":"heavy","keywords":["k"]}]}"#,
    "custom_dimensions.0.weight"
)]
#[case::keywords_type(r#"{"escalation_keywords":"nope"}"#, "escalation_keywords")]
#[case::margin_type(r#"{"hybrid_boundary_margin":"0.5"}"#, "hybrid_boundary_margin")]
#[case::window_type(r#"{"stall_escalation_window":-1}"#, "stall_escalation_window")]
fn a_config_write_reads_every_field_at_its_own_path(#[case] config: &str, #[case] expected: &str) {
    assert_eq!(rejected_location(config), expected);
}

#[test]
fn a_config_write_message_names_the_key_and_the_load() {
    let error = validate_complexity_router_config_write(Some(&json(
        r#"{"classifier_type":"hybrid","hybrid_boundary_margin":1.5}"#,
    )))
    .expect_err("the margin is out of range");
    assert_eq!(
        error.to_string(),
        "complexity_router_config is invalid at hybrid_boundary_margin: hybrid_boundary_margin 1.5 \
         must be between 0 and 1. The router would drop this deployment at load time, so the write \
         is rejected instead."
    );
    let unreadable =
        validate_complexity_router_config_write(Some(&json(r#"{"classifier_type":"nope"}"#)))
            .expect_err("the classifier type is unknown");
    let message = unreadable.to_string();
    assert!(message.starts_with("complexity_router_config is invalid at classifier_type: "));
    assert!(message.ends_with(
        "The router would drop this deployment at load time, so the write is rejected instead."
    ));
}

#[rstest]
#[case::list("[]")]
#[case::text(r#""nope""#)]
#[case::number("5")]
#[case::boolean("true")]
fn a_config_write_that_is_not_an_object_says_so(#[case] config: &str) {
    let error = validate_complexity_router_config_write(Some(&json(config)))
        .expect_err("a non-object config is rejected");
    assert!(matches!(error, ComplexityConfigWriteError::NotAnObject));
    assert_eq!(
        error.to_string(),
        "complexity_router_config must be an object; the router would drop this deployment at load \
         time, so the write is rejected instead."
    );
}

#[test]
fn a_config_write_accepts_the_edge_shapes_the_load_accepts() {
    for accepted in [
        r#"{"tiers":null,"plan_mode_min_tier":null,"adaptive":null}"#,
        r#"{"enable_non_reasoning_tier":false}"#,
        r#"{"classification_mode":"every_request"}"#,
        r#"{"plugins":[]}"#,
        r#"{"plugins":["pkg.Plugin"]}"#,
        r#"{"tiers":{"SIMPLE":["x",{"model_name":"y","litellm_params":{"temperature":0.2}}]}}"#,
        r#"{"future_key":1}"#,
        r#"{"context_compaction":false}"#,
        r#"{"custom_dimensions":[],"dimension_weights":{}}"#,
    ] {
        let judged = validate_complexity_router_config_write(Some(&json(accepted)));
        assert!(judged.is_ok(), "{accepted}: {judged:?}");
    }
}

#[test]
fn a_config_write_rule_error_keeps_the_rule_as_its_cause() {
    let error =
        validate_complexity_router_config_write(Some(&json(r#"{"plan_mode_min_tier":"NOPE"}"#)))
            .expect_err("the tier is unknown");
    match error {
        ComplexityConfigWriteError::Invalid { location, source } => {
            assert_eq!(location, "complexity_router_config");
            assert!(source.to_string().contains("plan_mode_min_tier"));
        }
        other => panic!("expected an invalid config, got {other:?}"),
    }
}
