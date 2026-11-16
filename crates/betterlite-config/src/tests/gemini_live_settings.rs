use std::collections::BTreeMap;

use serde::Deserialize;

use crate::loader::parse_view;
use crate::types::LiteLlmSettings;

const DEFER_SETUP_VARIABLE: &str = "LITELLM_GEMINI_LIVE_DEFER_SETUP";

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    gemini_live_defer_setup: Option<bool>,
}

// the loader's own view parses the key until the section registry carries an entry for it
fn configured(source: &str) -> Option<bool> {
    let (raw, _unknown) = parse_view::<Raw>(source).expect("the loader's view parses the key");
    raw.litellm_settings.gemini_live_defer_setup
}

fn resolved(source: &str, environment: &[(&str, &str)]) -> bool {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    LiteLlmSettings::gemini_live_defer_setup(configured(source), &lookup)
}

#[test]
fn loads_gemini_live_defer_setup_from_the_module_setting() {
    assert!(resolved(
        "litellm_settings:\n  gemini_live_defer_setup: true\n",
        &[]
    ));
    assert!(!resolved(
        "litellm_settings:\n  gemini_live_defer_setup: false\n",
        &[]
    ));
}

#[test]
fn loads_gemini_live_defer_setup_from_the_environment() {
    assert!(resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "true")]
    ));
    assert!(resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "TRUE")]
    ));
    assert!(resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "True")]
    ));
}

#[test]
fn loads_gemini_live_defer_setup_without_trimming() {
    assert!(!resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, " true")]
    ));
    assert!(!resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "true ")]
    ));
    assert!(!resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "yes")]
    ));
    assert!(!resolved(
        "model_list: []\n",
        &[(DEFER_SETUP_VARIABLE, "1")]
    ));
}

#[test]
fn loads_gemini_live_defer_setup_as_false_by_default() {
    assert!(!resolved("model_list: []\n", &[]));
    assert!(!resolved(
        "litellm_settings:\n  gemini_live_defer_setup: null\n",
        &[]
    ));
}

#[test]
fn loads_gemini_live_defer_setup_from_the_module_before_the_environment() {
    assert!(!resolved(
        "litellm_settings:\n  gemini_live_defer_setup: false\n",
        &[(DEFER_SETUP_VARIABLE, "true")]
    ));
    assert!(resolved(
        "litellm_settings:\n  gemini_live_defer_setup: true\n",
        &[(DEFER_SETUP_VARIABLE, "false")]
    ));
}
