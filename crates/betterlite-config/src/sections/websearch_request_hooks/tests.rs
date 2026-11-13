#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::websearch_request_hooks::Settings;

fn variables(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = variables(environment);
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.websearch_request_hooks
}

fn load_error(source: &str, environment: &[(&str, &str)]) -> String {
    let lookup = variables(environment);
    ConfigLoader { lookup }
        .load(source)
        .expect_err("the config fails to load")
        .to_string()
}

fn params(body: &str) -> String {
    format!("litellm_settings:\n  websearch_interception_params:\n{body}")
}

#[test]
fn the_provider_default_is_bedrock_when_the_key_is_absent() {
    let loaded = settings("litellm_settings: {}", &[]);
    assert_eq!(loaded.enabled_providers, ["bedrock"]);
    assert_eq!(loaded.search_tool_name, None);
    assert_eq!(loaded.max_agentic_loops, None);
}

#[rstest]
#[case::null("    enabled_providers: null\n")]
#[case::empty_mapping("")]
fn the_provider_default_applies_without_a_list(#[case] body: &str) {
    let loaded = settings(&params(body), &[]);
    assert_eq!(loaded.enabled_providers, ["bedrock"]);
}

#[test]
fn an_explicit_empty_provider_list_disables_every_provider() {
    let loaded = settings(&params("    enabled_providers: []\n"), &[]);
    assert_eq!(loaded.enabled_providers, Vec::<String>::new());
}

#[test]
fn a_provider_list_and_search_tool_name_roundtrip_through_yaml() {
    let loaded = settings(
        &params(
            "    enabled_providers: [\"bedrock\", \"vertex_ai\"]\n    search_tool_name: my-search\n",
        ),
        &[],
    );
    assert_eq!(loaded.enabled_providers, ["bedrock", "vertex_ai"]);
    assert_eq!(loaded.search_tool_name.as_deref(), Some("my-search"));
}

#[test]
fn the_callback_settings_entry_is_used_when_the_key_is_absent() {
    let loaded = settings(
        "callback_settings:\n  websearch_interception:\n    search_tool_name: callback-search\n",
        &[],
    );
    assert_eq!(loaded.search_tool_name.as_deref(), Some("callback-search"));
    assert_eq!(loaded.enabled_providers, ["bedrock"]);
}

#[test]
fn initialize_from_proxy_config_prefers_the_litellm_settings_entry() {
    let loaded = settings(
        "litellm_settings:\n  websearch_interception_params:\n    search_tool_name: settings-search\ncallback_settings:\n  websearch_interception:\n    search_tool_name: callback-search\n",
        &[],
    );
    assert_eq!(loaded.search_tool_name.as_deref(), Some("settings-search"));
}

#[test]
fn initialize_from_proxy_config_falls_back_to_the_callback_entry() {
    let loaded = settings(
        "litellm_settings:\n  websearch_interception_params: null\ncallback_settings:\n  websearch_interception:\n    search_tool_name: callback-search\n",
        &[],
    );
    assert_eq!(loaded.search_tool_name.as_deref(), Some("callback-search"));
}

#[test]
fn initialize_from_proxy_config_defaults_when_neither_entry_is_set() {
    let loaded = settings("litellm_settings: {}\ncallback_settings: {}\n", &[]);
    assert_eq!(loaded.enabled_providers, ["bedrock"]);
    assert_eq!(loaded.search_tool_name, None);
}

#[rstest]
#[case::true_value("true")]
#[case::false_value("false")]
fn ceiling_validation_rejects_a_bool(#[case] value: &str) {
    let error = load_error(&params(&format!("    max_agentic_loops: {value}\n")), &[]);
    assert!(
        error.contains("litellm_settings.websearch_interception_params.max_agentic_loops"),
        "{error}"
    );
}

#[rstest]
#[case::zero("0", "below the minimum")]
#[case::negative("-2", "below the minimum")]
#[case::fraction("1.5", "max_agentic_loops")]
#[case::text("five", "max_agentic_loops")]
fn ceiling_validation_rejects_a_value(#[case] value: &str, #[case] expected: &str) {
    let error = load_error(&params(&format!("    max_agentic_loops: {value}\n")), &[]);
    assert!(error.contains(expected), "{error}");
}

#[rstest]
#[case::integer("5", 5)]
#[case::minimum("1", 1)]
#[case::float("5.0", 5)]
#[case::text("'5'", 5)]
fn ceiling_validation_accepts_a_whole_number(#[case] value: &str, #[case] expected: u32) {
    let loaded = settings(&params(&format!("    max_agentic_loops: {value}\n")), &[]);
    assert_eq!(loaded.max_agentic_loops, Some(expected));
}

#[rstest]
#[case::absent("")]
#[case::null("    max_agentic_loops: null\n")]
fn ceiling_default_is_none(#[case] body: &str) {
    let loaded = settings(&params(body), &[]);
    assert_eq!(loaded.max_agentic_loops, None);
}

#[test]
fn an_environment_reference_supplies_the_ceiling() {
    let loaded = settings(
        &params("    max_agentic_loops: os.environ/MAX_LOOPS\n"),
        &[("MAX_LOOPS", "7")],
    );
    assert_eq!(loaded.max_agentic_loops, Some(7));
}

#[test]
fn an_empty_environment_reference_counts_as_unset() {
    let loaded = settings(
        &params("    max_agentic_loops: os.environ/MAX_LOOPS\n"),
        &[("MAX_LOOPS", "")],
    );
    assert_eq!(loaded.max_agentic_loops, None);
}

#[test]
fn a_missing_environment_reference_is_a_load_error() {
    let error = load_error(
        &params("    max_agentic_loops: os.environ/MAX_LOOPS\n"),
        &[],
    );
    assert!(error.contains("MAX_LOOPS"), "{error}");
}

#[test]
fn an_environment_reference_resolves_in_the_search_tool_name() {
    let loaded = settings(
        &params("    search_tool_name: os.environ/SEARCH_TOOL\n"),
        &[("SEARCH_TOOL", "env-search")],
    );
    assert_eq!(loaded.search_tool_name.as_deref(), Some("env-search"));
}

#[test]
fn a_non_list_provider_list_is_a_load_error() {
    let error = load_error(&params("    enabled_providers: bedrock\n"), &[]);
    assert!(!error.is_empty());
}

#[test]
fn a_list_under_the_callback_settings_entry_is_rejected() {
    let error = load_error(
        "callback_settings:\n  websearch_interception:\n    enabled_providers: [bedrock]\n",
        &[],
    );
    assert!(
        error.contains("callback_settings.websearch_interception.enabled_providers"),
        "{error}"
    );
}

#[test]
fn the_websearch_params_are_no_longer_unsupported() {
    let loaded = settings(&params("    search_tool_name: known\n"), &[]);
    assert_eq!(loaded.search_tool_name.as_deref(), Some("known"));
}
