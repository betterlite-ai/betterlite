#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;

use super::{
    DEFAULT_EMBEDDING_MODEL, DEFAULT_ON_FLAGGED_ACTION, SemanticGuardInitError,
    initialize_semantic_guard,
};
use crate::guardrail_initializers::error::InitializerError;
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;

fn params(body: &str) -> LitellmParams {
    decode_litellm_params(body.as_bytes()).expect("params")
}

fn guarded(fields: &str) -> LitellmParams {
    params(&format!(
        r#"{{"guardrail": "semantic_guard", "mode": "pre_call"{fields}}}"#
    ))
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn empty() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn the_name_is_required_before_the_router() {
    let error = initialize_semantic_guard(&guarded(""), None, false, &empty()).unwrap_err();
    assert!(matches!(error, SemanticGuardInitError::MissingName));
    assert_eq!(
        error.to_string(),
        "SemanticGuard: guardrail_name is required"
    );
}

#[test]
fn an_empty_name_is_missing() {
    let error = initialize_semantic_guard(&guarded(""), Some(""), true, &empty()).unwrap_err();
    assert!(matches!(error, SemanticGuardInitError::MissingName));
}

#[test]
fn the_router_is_required() {
    let error = initialize_semantic_guard(&guarded(""), Some("semantic-guard"), false, &empty())
        .unwrap_err();
    assert!(matches!(error, SemanticGuardInitError::MissingRouter));
    assert_eq!(
        error.to_string(),
        "SemanticGuard requires llm_router for embeddings. Configure a model_list with an embedding model."
    );
}

#[test]
fn the_defaults_fill_every_unset_field() {
    let built =
        initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &empty()).unwrap();
    assert_eq!(built.name, "semantic-guard");
    assert_eq!(built.embedding_model, DEFAULT_EMBEDDING_MODEL);
    assert_eq!(built.similarity_threshold.to_bits(), 0.75_f64.to_bits());
    assert!(built.route_templates.is_empty());
    assert_eq!(built.custom_routes_file, None);
    assert!(built.custom_routes.is_empty());
    assert_eq!(built.on_flagged_action, DEFAULT_ON_FLAGGED_ACTION);
    assert_eq!(
        built.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert!(!built.default_on);
}

#[rstest]
#[case::zero(0.0)]
#[case::negative_zero(-0.0)]
fn an_explicit_zero_threshold_takes_the_default(#[case] value: f64) {
    let fields = format!(r#", "similarity_threshold": {value}"#);
    let built =
        initialize_semantic_guard(&guarded(&fields), Some("semantic-guard"), true, &empty())
            .unwrap();
    assert_eq!(built.similarity_threshold.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn a_configured_threshold_is_kept() {
    let built = initialize_semantic_guard(
        &guarded(r#", "similarity_threshold": 0.4"#),
        Some("semantic-guard"),
        true,
        &empty(),
    )
    .unwrap();
    assert_eq!(built.similarity_threshold.to_bits(), 0.4_f64.to_bits());
}

#[test]
fn the_environment_threshold_is_the_default() {
    let variables = environment(&[("DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD", "0.6")]);
    let built =
        initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &variables).unwrap();
    assert_eq!(built.similarity_threshold.to_bits(), 0.6_f64.to_bits());
}

#[test]
fn a_zero_threshold_takes_the_environment_default() {
    let variables = environment(&[("DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD", "0.6")]);
    let built = initialize_semantic_guard(
        &guarded(r#", "similarity_threshold": 0"#),
        Some("semantic-guard"),
        true,
        &variables,
    )
    .unwrap();
    assert_eq!(built.similarity_threshold.to_bits(), 0.6_f64.to_bits());
}

#[test]
fn an_empty_environment_threshold_counts_as_unset() {
    let variables = environment(&[("DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD", "")]);
    let built =
        initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &variables).unwrap();
    assert_eq!(built.similarity_threshold.to_bits(), 0.75_f64.to_bits());
}

#[test]
fn an_invalid_environment_threshold_is_an_error() {
    let variables = environment(&[("DEFAULT_SEMANTIC_GUARD_SIMILARITY_THRESHOLD", "high")]);
    let error = initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &variables)
        .unwrap_err();
    assert!(
        matches!(
            &error,
            SemanticGuardInitError::Threshold(RouteLoadError::ThresholdEnv { .. })
        ),
        "unexpected error {error}"
    );
}

#[test]
fn the_environment_embedding_model_is_the_default() {
    let variables = environment(&[(
        "DEFAULT_SEMANTIC_GUARD_EMBEDDING_MODEL",
        "text-embedding-3-large",
    )]);
    let built =
        initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &variables).unwrap();
    assert_eq!(built.embedding_model, "text-embedding-3-large");
}

#[test]
fn an_empty_environment_embedding_model_counts_as_unset() {
    let variables = environment(&[("DEFAULT_SEMANTIC_GUARD_EMBEDDING_MODEL", "")]);
    let built =
        initialize_semantic_guard(&guarded(""), Some("semantic-guard"), true, &variables).unwrap();
    assert_eq!(built.embedding_model, DEFAULT_EMBEDDING_MODEL);
}

#[test]
fn a_configured_embedding_model_wins_over_the_environment() {
    let variables = environment(&[(
        "DEFAULT_SEMANTIC_GUARD_EMBEDDING_MODEL",
        "text-embedding-3-large",
    )]);
    let built = initialize_semantic_guard(
        &guarded(r#", "embedding_model": "text-embedding-ada-002""#),
        Some("semantic-guard"),
        true,
        &variables,
    )
    .unwrap();
    assert_eq!(built.embedding_model, "text-embedding-ada-002");
}

#[test]
fn an_explicit_empty_embedding_model_is_kept() {
    let built = initialize_semantic_guard(
        &guarded(r#", "embedding_model": """#),
        Some("semantic-guard"),
        true,
        &empty(),
    )
    .unwrap();
    assert_eq!(built.embedding_model, "");
}

#[test]
fn the_route_settings_copy_through() {
    let fields = concat!(
        r#", "route_templates": ["sql_injection"],"#,
        r#" "custom_routes_file": "/etc/litellm/routes.yaml","#,
        r#" "custom_routes": [{"route_name": "attack", "utterances": ["x"]}],"#,
        r#" "on_flagged_action": "passthrough""#
    );
    let built = initialize_semantic_guard(&guarded(fields), Some("semantic-guard"), true, &empty())
        .unwrap();
    assert_eq!(built.route_templates, vec!["sql_injection".to_owned()]);
    assert_eq!(
        built.custom_routes_file,
        Some(PathBuf::from("/etc/litellm/routes.yaml"))
    );
    assert_eq!(built.custom_routes.len(), 1);
    assert_eq!(built.on_flagged_action, "passthrough");
}

#[test]
fn default_on_is_copied() {
    let built = initialize_semantic_guard(
        &guarded(r#", "default_on": true"#),
        Some("semantic-guard"),
        true,
        &empty(),
    )
    .unwrap();
    assert!(built.default_on);
}

#[test]
fn nulls_read_as_absent() {
    let fields = concat!(
        r#", "embedding_model": null, "similarity_threshold": null,"#,
        r#" "route_templates": null, "custom_routes_file": null,"#,
        r#" "custom_routes": null, "on_flagged_action": null, "default_on": null"#
    );
    let built = initialize_semantic_guard(&guarded(fields), Some("semantic-guard"), true, &empty())
        .unwrap();
    assert_eq!(built.embedding_model, DEFAULT_EMBEDDING_MODEL);
    assert_eq!(built.similarity_threshold.to_bits(), 0.75_f64.to_bits());
    assert!(built.route_templates.is_empty());
    assert_eq!(built.custom_routes_file, None);
    assert!(built.custom_routes.is_empty());
    assert_eq!(built.on_flagged_action, DEFAULT_ON_FLAGGED_ACTION);
    assert!(!built.default_on);
}

#[test]
fn the_mode_is_the_event_hook() {
    let decoded = params(r#"{"guardrail": "semantic_guard", "mode": "post_call"}"#);
    let built =
        initialize_semantic_guard(&decoded, Some("semantic-guard"), true, &empty()).unwrap();
    assert_eq!(
        built.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[rstest]
#[case::embedding_model(r#""embedding_model": 3"#, "embedding_model")]
#[case::similarity_threshold(r#""similarity_threshold": "high""#, "similarity_threshold")]
#[case::route_templates(r#""route_templates": "sql_injection""#, "route_templates")]
#[case::custom_routes_file(r#""custom_routes_file": 3"#, "custom_routes_file")]
#[case::custom_routes(r#""custom_routes": "attack""#, "custom_routes")]
#[case::on_flagged_action(r#""on_flagged_action": 3"#, "on_flagged_action")]
fn a_wrong_type_names_the_field(#[case] field: &str, #[case] name: &str) {
    let fields = format!(", {field}");
    let error =
        initialize_semantic_guard(&guarded(&fields), Some("semantic-guard"), true, &empty())
            .unwrap_err();
    assert!(
        matches!(
            &error,
            SemanticGuardInitError::Params(InitializerError::InvalidParam(reported)) if reported == name
        ),
        "unexpected error {error}"
    );
}
