use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

const DEFER_SETUP_VARIABLE: &str = "LITELLM_GEMINI_LIVE_DEFER_SETUP";

fn resolved(source: &str, environment: &[(&str, &str)]) -> bool {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("the loader accepts the gemini live defer setting");
    config.litellm_settings.gemini_live_defer_setup
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
