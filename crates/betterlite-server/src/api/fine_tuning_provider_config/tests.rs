#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::api::fine_tuning_provider_config::{
    FineTuningConfigError, Settings as FineTuningSettings,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;

use super::FineTuningProviderConfig;

fn environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_TEST".to_owned(), "first".to_owned());
    environment
}

fn loaded(source: &str) -> FineTuningSettings {
    ConfigLoader {
        lookup: environment(),
    }
    .load(source)
    .expect("the config loads")
    .sections
    .fine_tuning_provider_config
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(entries) = value else {
        panic!("the entry is an object");
    };
    entries.get(name).expect("the entry has the field")
}

fn two_azure_entries() -> FineTuningSettings {
    loaded(
        "finetune_settings:\n  - custom_llm_provider: azure\n    api_key: os.environ/AZURE_TEST\n  - custom_llm_provider: azure\n    api_key: second\n",
    )
}

#[test]
fn the_startup_config_keeps_provider_order_and_resolves_shallow_secrets() {
    let config = FineTuningProviderConfig::new(&two_azure_entries());
    let azure = config
        .provider("azure")
        .expect("the startup config is installed")
        .expect("azure is configured");
    assert_eq!(field(&azure, "api_key"), &text("first"));
    assert!(
        config
            .provider("openai")
            .expect("the startup config is installed")
            .is_none()
    );
}

#[test]
fn a_null_set_keeps_the_installed_configuration() {
    let config = FineTuningProviderConfig::new(&two_azure_entries());
    let environment = environment();
    config
        .set(Some(&Json::Null), &environment)
        .expect("a null set is accepted");
    let azure = config
        .provider("azure")
        .expect("the config stays installed")
        .expect("azure stays configured");
    assert_eq!(field(&azure, "api_key"), &text("first"));
}

#[test]
fn a_set_replaces_the_configuration_for_every_shared_snapshot() {
    let config = FineTuningProviderConfig::new(&FineTuningSettings::default());
    let shared = config.clone();
    let replacement = Json::List(vec![Json::Object(indexmap::IndexMap::from([(
        "custom_llm_provider".to_owned(),
        text("openai"),
    )]))]);
    let environment = environment();
    config
        .set(Some(&replacement), &environment)
        .expect("the list sets");
    assert!(
        shared
            .provider("openai")
            .expect("the shared snapshot sees the set")
            .is_some()
    );
    assert!(
        config
            .provider("azure")
            .expect("the shared snapshot sees the set")
            .is_none()
    );
}

#[test]
fn an_empty_list_is_configured_but_answers_null() {
    let config = FineTuningProviderConfig::new(&loaded("finetune_settings: []\n"));
    assert!(
        config
            .provider("azure")
            .expect("the empty list is configured")
            .is_none()
    );
}

#[test]
fn a_lookup_before_any_config_names_the_config_file() {
    let config = FineTuningProviderConfig::new(&FineTuningSettings::default());
    let error = config.provider("azure").expect_err("nothing is configured");
    assert!(matches!(error, FineTuningConfigError::Unset));
    assert_eq!(
        error.to_string(),
        "fine_tuning_config is not set, set it on your config.yaml file."
    );
}

#[test]
fn a_non_dict_entry_fails_a_lookup_that_reaches_it() {
    let config = FineTuningProviderConfig::new(&loaded(
        "finetune_settings:\n  - not-a-provider-map\n  - custom_llm_provider: azure\n",
    ));
    let error = config
        .provider("azure")
        .expect_err("the lookup reaches the string entry");
    assert!(matches!(
        error,
        FineTuningConfigError::NotAProviderMap { kind: "str" }
    ));
}

#[test]
fn a_non_list_set_is_rejected_and_keeps_the_previous_list() {
    let config = FineTuningProviderConfig::new(&two_azure_entries());
    let environment = environment();
    let error = config
        .set(Some(&text("azure")), &environment)
        .expect_err("a string is not a list");
    assert!(matches!(error, FineTuningConfigError::NotAList));
    assert!(
        config
            .provider("azure")
            .expect("the previous list stays installed")
            .is_some()
    );
}

#[test]
fn a_set_resolves_only_top_level_shallow_secrets() {
    let config = FineTuningProviderConfig::new(&FineTuningSettings::default());
    let environment = environment();
    let replacement = Json::List(vec![Json::Object(indexmap::IndexMap::from([
        ("custom_llm_provider".to_owned(), text("azure")),
        ("api_key".to_owned(), text("os.environ/AZURE_TEST")),
        (
            "metadata".to_owned(),
            Json::Object(indexmap::IndexMap::from([(
                "token".to_owned(),
                text("os.environ/AZURE_TEST"),
            )])),
        ),
    ]))]);
    config
        .set(Some(&replacement), &environment)
        .expect("the list sets");
    let azure = config
        .provider("azure")
        .expect("the list is installed")
        .expect("azure is configured");
    assert_eq!(field(&azure, "api_key"), &text("first"));
    assert_eq!(
        field(&azure, "metadata"),
        &Json::Object(indexmap::IndexMap::from([(
            "token".to_owned(),
            text("os.environ/AZURE_TEST"),
        )]))
    );
}
