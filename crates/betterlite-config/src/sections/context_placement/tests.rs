#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::context_placement::{Config, DEFAULT_BUFFER, DEFAULT_ENABLED, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.context_placement)
        .map_err(|error| error.to_string())
}

fn router(fields: &str) -> String {
    format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n{fields}"
    )
}

#[test]
fn a_deployment_without_the_key_keeps_its_index() {
    let source =
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![None],
        }
    );
}

#[test]
fn an_explicit_null_takes_the_declared_default() {
    let source = router("        enable_context_window_escalation: null\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![Some(Config::default())],
        }
    );
}

#[test]
fn an_empty_router_config_resolves_to_the_defaults() {
    let source = "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config: {}\n";
    let resolved = settings(source, &[]).unwrap();
    assert_eq!(
        resolved,
        Settings {
            deployments: vec![Some(Config::default())],
        }
    );
}

#[test]
fn declared_values_replace_the_defaults() {
    let source = router(
        "        enable_context_window_escalation: true\n        context_window_escalation_buffer: 0.5\n",
    );
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved.deployments,
        vec![Some(Config {
            enabled: true,
            buffer: 0.5,
        })]
    );
}

#[test]
fn the_defaults_are_the_pinned_litellm_values() {
    assert_eq!(Config::default().enabled, DEFAULT_ENABLED);
    assert_eq!(Config::default().buffer.to_bits(), DEFAULT_BUFFER.to_bits());
    assert_eq!(DEFAULT_BUFFER.to_bits(), 0.95_f64.to_bits());
    assert!(!DEFAULT_ENABLED);
}

#[test]
fn one_buffer_at_the_upper_bound_is_accepted() {
    let source = router("        context_window_escalation_buffer: 1\n");
    let resolved = settings(&source, &[]).unwrap();
    assert_eq!(
        resolved.deployments[0].as_ref().unwrap().buffer.to_bits(),
        1.0_f64.to_bits()
    );
}

#[test]
fn one_buffer_above_the_upper_bound_is_rejected() {
    let source = router("        context_window_escalation_buffer: 1.001\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn a_zero_buffer_is_rejected() {
    let source = router("        context_window_escalation_buffer: 0\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn a_negative_buffer_is_rejected() {
    let source = router("        context_window_escalation_buffer: -0.1\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn a_boolean_buffer_is_rejected() {
    let source = router("        context_window_escalation_buffer: true\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn a_quoted_buffer_is_rejected() {
    let source = router("        context_window_escalation_buffer: \"0.5\"\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn a_non_numeric_buffer_is_rejected() {
    let source = router("        context_window_escalation_buffer: many\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn an_environment_buffer_resolves_before_the_range_check() {
    let source = router("        context_window_escalation_buffer: os.environ/WINDOW\n");
    let resolved = settings(&source, &[("WINDOW", "0.8")]).unwrap();
    assert_eq!(
        resolved.deployments[0].as_ref().unwrap().buffer.to_bits(),
        0.8_f64.to_bits()
    );
}

#[test]
fn an_environment_buffer_out_of_range_is_rejected() {
    let source = router("        context_window_escalation_buffer: os.environ/WINDOW\n");
    let error = settings(&source, &[("WINDOW", "2")]).unwrap_err();
    assert!(
        error.contains("context_window_escalation_buffer"),
        "{error}"
    );
}

#[test]
fn an_empty_environment_buffer_takes_the_default() {
    let source = router("        context_window_escalation_buffer: os.environ/WINDOW\n");
    let resolved = settings(&source, &[("WINDOW", "")]).unwrap();
    assert_eq!(
        resolved.deployments[0].as_ref().unwrap().buffer.to_bits(),
        0.95_f64.to_bits()
    );
}

#[test]
fn a_missing_environment_buffer_is_an_error() {
    let source = router("        context_window_escalation_buffer: os.environ/WINDOW\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("WINDOW"), "{error}");
}

#[test]
fn an_environment_flag_resolves_to_a_bool() {
    let source = router("        enable_context_window_escalation: os.environ/ESCALATE\n");
    let resolved = settings(&source, &[("ESCALATE", "yes")]).unwrap();
    assert!(resolved.deployments[0].as_ref().unwrap().enabled);
}

#[test]
fn an_empty_environment_flag_takes_the_default() {
    let source = router("        enable_context_window_escalation: os.environ/ESCALATE\n");
    let resolved = settings(&source, &[("ESCALATE", "")]).unwrap();
    assert!(!resolved.deployments[0].as_ref().unwrap().enabled);
}

#[test]
fn a_flag_of_explicit_false_stays_off() {
    let source = router("        enable_context_window_escalation: \"off\"\n");
    let resolved = settings(&source, &[]).unwrap();
    assert!(!resolved.deployments[0].as_ref().unwrap().enabled);
}

#[test]
fn a_flag_of_an_unknown_word_is_an_error() {
    let source = router("        enable_context_window_escalation: maybe\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(
        error.contains("enable_context_window_escalation")
            || error.contains("ESCALATE")
            || error.contains("bool"),
        "{error}"
    );
}

#[test]
fn a_numeric_flag_is_rejected() {
    let source = router("        enable_context_window_escalation: 1\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("RawFlag"), "{error}");
}

#[test]
fn a_sibling_keys_do_not_become_unsupported() {
    let source = router(
        "        enable_context_window_escalation: true\n        default_model: openai/gpt-4o\n",
    );
    let resolved = settings(&source, &[]).unwrap();
    assert!(resolved.deployments[0].as_ref().unwrap().enabled);
}

#[test]
fn an_unclaimed_nested_key_is_still_unsupported() {
    let source = router("        escalation_mystery: true\n");
    let error = settings(&source, &[]).unwrap_err();
    assert!(error.contains("escalation_mystery"), "{error}");
}
