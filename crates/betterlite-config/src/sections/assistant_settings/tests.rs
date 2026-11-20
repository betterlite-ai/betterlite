#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::assistant_settings::{Config, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .assistant_settings)
}

fn params(settings: Settings) -> BTreeMap<String, Json> {
    settings
        .config
        .expect("the assistant is configured")
        .litellm_params
        .expect("the parameters are configured")
}

#[test]
fn loads_assistant_settings() {
    for source in [
        "model_list: []",
        "assistant_settings:",
        "assistant_settings: null",
        "assistant_settings: {}",
    ] {
        let loaded = settings(source, &[]).expect("an unconfigured assistant loads");
        assert_eq!(loaded, Settings::default(), "{source}");
    }
    let source = "assistant_settings:\n  custom_llm_provider: openai\n  litellm_params:\n    api_key: sk-config\n    model: gpt-4o-mini\n";
    let loaded = settings(source, &[]).expect("a configured assistant loads");
    let config: Config = loaded.config.expect("the assistant is configured");
    assert_eq!(config.custom_llm_provider.as_deref(), Some("openai"));
    let params = config.litellm_params.expect("the parameters are loaded");
    assert_eq!(
        params.get("api_key"),
        Some(&Json::Text("sk-config".to_owned()))
    );
    assert_eq!(
        params.get("model"),
        Some(&Json::Text("gpt-4o-mini".to_owned()))
    );
}

#[test]
fn expands_only_top_level_environment_strings() {
    let source = "assistant_settings:\n  custom_llm_provider: openai\n  litellm_params:\n    api_key: os.environ/Aos.environ/B\n    model: os.environ/EMPTY\n    api_base: os.environ/MISSING\n    organization: plain\n    extra_body:\n      api_key: os.environ/UNTOUCHED\n";
    let loaded =
        settings(source, &[("AB", "combined"), ("EMPTY", "")]).expect("the environment resolves");
    let params = params(loaded);
    assert_eq!(
        params.get("api_key"),
        Some(&Json::Text("combined".to_owned()))
    );
    assert_eq!(params.get("model"), Some(&Json::Text(String::new())));
    assert_eq!(params.get("api_base"), Some(&Json::Null));
    assert_eq!(
        params.get("organization"),
        Some(&Json::Text("plain".to_owned()))
    );
    let nested = match params.get("extra_body") {
        Some(Json::Object(fields)) => fields.get("api_key"),
        Some(_) => panic!("extra_body should be a mapping"),
        None => None,
    };
    assert_eq!(nested, Some(&Json::Text("os.environ/UNTOUCHED".to_owned())));
}

#[test]
fn a_nonempty_mapping_without_params_fails_to_load() {
    let error = settings("assistant_settings:\n  custom_llm_provider: openai\n", &[])
        .expect_err("the mapping is missing litellm_params");
    assert!(
        format!("{error}").contains("litellm_params"),
        "the missing key is named: {error}"
    );
}

#[test]
fn a_mapping_without_a_provider_loads_for_the_dispatcher() {
    let loaded =
        settings("assistant_settings:\n  litellm_params:\n    api_key: sk-config\n", &[])
            .expect("only the factory reads custom_llm_provider");
    let config = loaded.config.expect("the assistant is configured");
    assert_eq!(config.custom_llm_provider, None);
    assert_eq!(
        config
            .litellm_params
            .expect("the parameters are loaded")
            .get("api_key"),
        Some(&Json::Text("sk-config".to_owned()))
    );
}
