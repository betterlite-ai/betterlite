#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::request_body_safety::{ClientsideParamGrant, DEFAULT_MAX_FALLBACKS, Settings};

fn load(yaml: &str) -> Result<Settings, ConfigError> {
    load_with(yaml, &[])
}

fn load_with(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(yaml)
        .map(|config| config.sections.request_body_safety)
}

fn parsed(yaml: &str) -> Settings {
    load(yaml).expect("the config loads")
}

#[test]
fn credential_opt_in_default_and_literal_true() {
    assert!(!parsed("{}").allow_client_side_credentials);
    assert!(
        !parsed("general_settings: {allow_client_side_credentials: false}")
            .allow_client_side_credentials
    );
    assert!(
        !parsed("general_settings: {allow_client_side_credentials: null}")
            .allow_client_side_credentials
    );
    assert!(
        parsed("general_settings: {allow_client_side_credentials: true}")
            .allow_client_side_credentials
    );
    // a string is not the bool the module compares with `is True`; the typed load refuses it
    assert!(matches!(
        load("general_settings: {allow_client_side_credentials: 'true'}"),
        Err(ConfigError::Yaml { .. })
    ));
}

#[test]
fn model_param_grants_load() {
    let yaml = "\
model_list:
  - model_name: grant-model
    litellm_params:
      configurable_clientside_auth_params: [aws_role_name, {api_base: '^https://ok.example'}]
  - model_name: unset-model
    litellm_params: {model: openai/gpt-4o}
  - model_name: grant-model
    litellm_params: {model: openai/gpt-4o, configurable_clientside_auth_params: [aws_session_name]}
";
    let settings = parsed(yaml);
    let grants = settings
        .model_grants
        .get("grant-model")
        .expect("the model loads");
    let items = grants.as_ref().expect("the last duplicate deployment wins");
    assert_eq!(
        items.first(),
        Some(&ClientsideParamGrant::Exact("aws_session_name".to_owned()))
    );
    assert_eq!(settings.model_grants.get("unset-model"), Some(&None));

    let object_yaml = "\
model_list:
  - model_name: matcher-model
    litellm_params:
      configurable_clientside_auth_params:
        - {api_base: '^https://ok.example'}
";
    let matcher = parsed(object_yaml)
        .model_grants
        .remove("matcher-model")
        .flatten()
        .expect("the matcher loads");
    let ClientsideParamGrant::Matcher(fields) = &matcher[0] else {
        panic!("a mapping is an api_base matcher");
    };
    assert_eq!(
        fields.get("api_base"),
        Some(&Json::Text("^https://ok.example".to_owned()))
    );

    // LiteLLM's pydantic model allows only strings and mappings in the list
    let error = load(
        "model_list:\n  - model_name: broken\n    litellm_params: {configurable_clientside_auth_params: [7]}\n",
    )
    .expect_err("a number is not a grant");
    assert!(
        matches!(error, ConfigError::InvalidScalar { ref path, .. } if path.contains("configurable_clientside_auth_params")),
        "{error:?}"
    );
}

#[test]
fn allowed_hosts_load_without_bypass() {
    let settings = parsed(
        "litellm_settings:\n  provider_url_destination_allowed_hosts: [https://allowed.example]\n",
    );
    assert_eq!(settings.allowed_hosts.len(), 1);
    assert_eq!(settings.allowed_hosts[0].host, "allowed.example");
    // loading the allowlist never turns on the credential opt-in that could authorize the same field
    assert!(!settings.allow_client_side_credentials);

    let resolved = load_with(
        "litellm_settings:\n  provider_url_destination_allowed_hosts: [os.environ/ALLOWED_HOST]\n",
        &[("ALLOWED_HOST", "resolved.example")],
    )
    .expect("the env reference resolves");
    assert_eq!(resolved.allowed_hosts[0].host, "resolved.example");

    let absent = load_with(
        "litellm_settings:\n  provider_url_destination_allowed_hosts: [os.environ/UNSET_HOST]\n",
        &[],
    )
    .expect_err("a missing env reference stops the load");
    let ConfigError::MissingEnvironment { path, .. } = &absent else {
        panic!("a missing env reference reports its path, got {absent:?}");
    };
    assert_eq!(
        path,
        "litellm_settings.provider_url_destination_allowed_hosts"
    );
}

#[test]
fn fallback_depth_default_override() {
    assert_eq!(parsed("{}").max_fallbacks, DEFAULT_MAX_FALLBACKS);
    let overridden = load_with("{}", &[("ROUTER_MAX_FALLBACKS", "9")]).expect("the number loads");
    assert_eq!(overridden.max_fallbacks, 9);
    // the shared env rule reads an empty value as unset
    let empty = load_with("{}", &[("ROUTER_MAX_FALLBACKS", "")]).expect("empty is unset");
    assert_eq!(empty.max_fallbacks, DEFAULT_MAX_FALLBACKS);
}

#[test]
fn fallback_depth_bad_integer_startup() {
    let error = load_with("{}", &[("ROUTER_MAX_FALLBACKS", "five")])
        .expect_err("int() at import fails on text");
    assert!(
        matches!(error, ConfigError::Environment { .. }),
        "{error:?}"
    );
}

#[test]
fn fallback_depth_nonpositive() {
    let zero = load_with("{}", &[("ROUTER_MAX_FALLBACKS", "0")]).expect("zero loads");
    assert_eq!(zero.max_fallbacks, 0);
    let negative = load_with("{}", &[("ROUTER_MAX_FALLBACKS", "-3")]).expect("a negative loads");
    assert_eq!(negative.max_fallbacks, -3);
}
