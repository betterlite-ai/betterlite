#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::capability_forecast::{Calibration, Config, ResponseFormat, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.capability_forecast)
        .map_err(|error| error.to_string())
}

fn deployment(params: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n{params}"
    )
}

fn capability(fields: &str) -> String {
    router_config(&format!("        capability_classifier_config:\n{fields}"))
}

fn router_config(fields: &str) -> String {
    deployment(&format!("      complexity_router_config:\n{fields}"))
}

#[test]
fn a_deployment_without_the_key_keeps_its_index() {
    let source = deployment("      timeout: 5\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![None],
        }
    );
}

#[test]
fn a_full_configuration_resolves_every_key() {
    let source = capability(
        "          efficient_tier: SIMPLE\n          capable_tier: REASONING\n          base_threshold: 0.6\n          threshold_step: 0.05\n          max_output_tokens: 512\n          response_format: json_object\n          calibration:\n            version: held-out-v1\n            slope: 1.5\n            intercept: -0.25\n",
    );
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![Some(Config {
                efficient_tier: "SIMPLE".to_owned(),
                capable_tier: "REASONING".to_owned(),
                base_threshold: 0.6_f64,
                threshold_step: 0.05_f64,
                max_output_tokens: 512,
                calibration: Some(Calibration {
                    version: "held-out-v1".to_owned(),
                    slope: 1.5_f64,
                    intercept: -0.25_f64,
                }),
                response_format: ResponseFormat::JsonObject,
            })],
        }
    );
}

#[test]
fn only_the_required_key_keeps_the_declared_defaults() {
    let source = capability("          base_threshold: 0.5\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(Config {
            efficient_tier: "SIMPLE".to_owned(),
            capable_tier: "REASONING".to_owned(),
            base_threshold: 0.5_f64,
            threshold_step: 0.0_f64,
            max_output_tokens: 4096,
            calibration: None,
            response_format: ResponseFormat::JsonSchema,
        })]
    );
}

#[test]
fn environment_references_resolve_once_at_load() {
    let source = capability(
        "          efficient_tier: os.environ/EFFICIENT_TIER\n          base_threshold: os.environ/BASE_THRESHOLD\n          threshold_step: os.environ/THRESHOLD_STEP\n          calibration:\n            version: os.environ/CALIBRATION_VERSION\n            slope: os.environ/SLOPE\n            intercept: os.environ/INTERCEPT\n",
    );
    let resolved = settings(
        &source,
        &[
            ("EFFICIENT_TIER", "MEDIUM"),
            ("BASE_THRESHOLD", "0.25"),
            ("THRESHOLD_STEP", "0.125"),
            ("CALIBRATION_VERSION", "v2"),
            ("SLOPE", "2"),
            ("INTERCEPT", "-1"),
        ],
    )
    .unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(Config {
            efficient_tier: "MEDIUM".to_owned(),
            capable_tier: "REASONING".to_owned(),
            base_threshold: 0.25_f64,
            threshold_step: 0.125_f64,
            max_output_tokens: 4096,
            calibration: Some(Calibration {
                version: "v2".to_owned(),
                slope: 2.0_f64,
                intercept: -1.0_f64,
            }),
            response_format: ResponseFormat::JsonSchema,
        })]
    );
}

#[rstest]
#[case::configured_object(
    "      complexity_router_config:\n        capability_classifier_config:\n          base_threshold: 0.5\n"
)]
#[case::another_deployment("      timeout: 5\n")]
fn one_setting_leaves_the_other_deployment_absent(#[case] params: &str) {
    let source = format!(
        "model_list:\n  - model_name: first\n    litellm_params:\n      model: openai/gpt-4o\n{params}  - model_name: second\n    litellm_params:\n      model: openai/gpt-4o\n"
    );
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 2);
    assert_eq!(
        resolved.deployments[0].is_some(),
        params.contains("base_threshold")
    );
    assert_eq!(resolved.deployments[1], None);
}

#[rstest]
#[case::missing_base_threshold(
    "        capability_classifier_config:\n          threshold_step: 0.2\n"
)]
#[case::unknown_inner_key(
    "        capability_classifier_config:\n          base_threshold: 0.5\n          threshold_steps: 0.2\n"
)]
#[case::unquoted_response_format(
    "        capability_classifier_config:\n          base_threshold: 0.5\n          response_format: json\n"
)]
#[case::text_base_threshold(
    "        capability_classifier_config:\n          base_threshold: soon\n"
)]
#[case::quoted_base_threshold(
    "        capability_classifier_config:\n          base_threshold: \"0.5\"\n"
)]
#[case::quoted_threshold_step(
    "        capability_classifier_config:\n          base_threshold: 0.5\n          threshold_step: \"0.1\"\n"
)]
#[case::quoted_calibration_slope(
    "        capability_classifier_config:\n          base_threshold: 0.5\n          calibration:\n            version: v1\n            slope: \"1.0\"\n            intercept: 0.0\n"
)]
fn rejects_configurations_lite_proxy_would_refuse(#[case] fields: &str) {
    let source = router_config(fields);
    let error = settings(&source, &[]).unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn the_router_siblings_lite_llm_requires_are_accepted() {
    let source = router_config(
        "        classifier_type: capability\n        classifier_llm_config:\n          model: judge-model\n          timeout_ms: 400\n        tiers:\n          SIMPLE: efficient-model\n          REASONING: capable-model\n        undeclared_router_key: true\n        capability_classifier_config:\n          base_threshold: 0.5\n",
    );
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(resolved.deployments.len(), 1);
    assert!(resolved.deployments[0].is_some());
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/capability-forecast-valid.yaml");
    let resolved = settings(source, &[]).unwrap();
    let Some(configured) = resolved.deployments.first().and_then(Option::as_ref) else {
        panic!("the shipped seed configures the capability classifier");
    };
    assert_eq!(configured.efficient_tier, "SIMPLE");
    assert_eq!(configured.capable_tier, "REASONING");
    assert_eq!(configured.base_threshold.to_bits(), 0.5_f64.to_bits());
    assert_eq!(configured.threshold_step.to_bits(), 0.1_f64.to_bits());
    assert_eq!(configured.max_output_tokens, 512);
    assert_eq!(configured.response_format, ResponseFormat::JsonObject);
    assert_eq!(
        configured.calibration,
        Some(Calibration {
            version: "held-out-v1".to_owned(),
            slope: 1.0_f64,
            intercept: 0.0_f64,
        })
    );
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/capability-forecast-invalid.yaml");
    let error = settings(source, &[]).unwrap_err();
    assert!(!error.is_empty());
}
