#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{FineTuningConfigError, fine_tuning_config_for, set_fine_tuning_config};
use crate::loader::{ConfigError, ConfigLoader};

fn environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    environment.insert("AZURE_TEST".to_owned(), "first".to_owned());
    environment
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn entry(provider: &str, api_key: Json) -> Json {
    Json::Object(indexmap::IndexMap::from([
        ("custom_llm_provider".to_owned(), text(provider)),
        ("api_key".to_owned(), api_key),
    ]))
}

fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(entries) = value else {
        panic!("the entry is an object");
    };
    entries.get(name).expect("the entry has the field")
}

#[test]
fn loads_finetune_settings() {
    let environment = environment();
    let config = ConfigLoader { lookup: environment }
        .load(
            "finetune_settings:\n  - custom_llm_provider: azure\n    api_key: os.environ/AZURE_TEST\n    api_version: \"2024-05-01-preview\"\n  - custom_llm_provider: openai\n    api_key: second\n",
        )
        .expect("finetune_settings load");
    let settings = config
        .sections
        .fine_tuning_provider_config
        .finetune_settings
        .expect("finetune_settings is configured");
    assert_eq!(settings.len(), 2);
    assert_eq!(field(&settings[0], "api_key"), &text("first"));
    assert_eq!(
        field(&settings[0], "api_version"),
        &text("2024-05-01-preview")
    );
    assert_eq!(field(&settings[1], "api_key"), &text("second"));
    let azure = fine_tuning_config_for(Some(&settings), "azure")
        .expect("the settings are set")
        .expect("azure is configured");
    assert_eq!(field(&azure, "api_key"), &text("first"));
}

#[test]
fn loads_an_unset_finetune_settings_as_null() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list: []\n")
    .expect("an empty config loads");
    assert!(
        config
            .sections
            .fine_tuning_provider_config
            .finetune_settings
            .is_none()
    );
}

#[test]
fn loads_a_null_finetune_settings_as_null() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("finetune_settings:\n")
    .expect("a null finetune_settings loads");
    assert!(
        config
            .sections
            .fine_tuning_provider_config
            .finetune_settings
            .is_none()
    );
}

#[test]
fn loads_an_empty_finetune_settings_list() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("finetune_settings: []\n")
    .expect("an empty list loads");
    let settings = config
        .sections
        .fine_tuning_provider_config
        .finetune_settings
        .expect("the empty list is configured");
    assert!(settings.is_empty());
    assert!(
        fine_tuning_config_for(Some(&settings), "azure")
            .expect("the empty list is set")
            .is_none()
    );
}

#[test]
fn keeps_nested_and_plain_strings_untouched() {
    let environment = environment();
    let config = ConfigLoader { lookup: environment }
        .load(
            "finetune_settings:\n  - custom_llm_provider: azure\n    api_key: os.environ/AZURE_TEST\n    metadata: {token: os.environ/AZURE_TEST}\n    note: plain-value\n",
        )
        .expect("finetune_settings load");
    let settings = config
        .sections
        .fine_tuning_provider_config
        .finetune_settings
        .expect("finetune_settings is configured");
    let nested = field(&settings[0], "metadata");
    let Json::Object(metadata) = nested else {
        panic!("metadata is an object");
    };
    assert_eq!(
        metadata.get("token"),
        Some(&text("os.environ/AZURE_TEST")),
        "only top-level values are resolved"
    );
    assert_eq!(field(&settings[0], "note"), &text("plain-value"));
}

#[test]
fn keeps_provider_order_and_returns_the_first_match() {
    let environment = environment();
    let settings = Json::List(vec![
        entry("azure", text("os.environ/AZURE_TEST")),
        entry("azure", text("second")),
    ]);
    let resolved = set_fine_tuning_config(None, Some(&settings), &environment)
        .expect("the list sets")
        .expect("the list is configured");
    let matched = fine_tuning_config_for(Some(&resolved), "azure")
        .expect("the settings are set")
        .expect("azure is configured");
    assert_eq!(field(&matched, "api_key"), &text("first"));
    assert_eq!(matched, resolved[0], "the whole first map is returned");
}

#[test]
fn stores_a_non_dict_entry_and_fails_a_lookup_that_reaches_it() {
    let environment = environment();
    let settings = Json::List(vec![text("not-a-provider"), entry("azure", text("first"))]);
    let resolved = set_fine_tuning_config(None, Some(&settings), &environment)
        .expect("the list stores as given")
        .expect("the list is configured");
    assert_eq!(resolved[0], text("not-a-provider"));
    let error = fine_tuning_config_for(Some(&resolved), "azure")
        .expect_err("the lookup reaches the string entry");
    assert!(matches!(
        error,
        FineTuningConfigError::NotAProviderMap { kind: "str" }
    ));
    assert_eq!(error.to_string(), "'str' object has no attribute 'get'");
}

#[test]
fn a_number_entry_names_an_int_or_a_float_as_python_does() {
    let environment = environment();
    let integer = Json::Number(serde_json::Number::from(1_u8));
    let float = Json::Number(serde_json::Number::from_f64(1.5).expect("a finite float"));
    let settings = Json::List(vec![integer, float]);
    let resolved = set_fine_tuning_config(None, Some(&settings), &environment)
        .expect("the list stores as given")
        .expect("the list is configured");
    let integer_error = fine_tuning_config_for(Some(&resolved), "azure")
        .expect_err("the lookup reaches the integer entry");
    assert_eq!(
        integer_error.to_string(),
        "'int' object has no attribute 'get'"
    );
    let float_error = fine_tuning_config_for(Some(&resolved[1..]), "azure")
        .expect_err("the lookup reaches the float entry");
    assert_eq!(
        float_error.to_string(),
        "'float' object has no attribute 'get'"
    );
}

#[test]
fn returns_null_for_a_provider_that_is_not_configured() {
    let environment = environment();
    let settings = Json::List(vec![entry("openai", text("second"))]);
    let resolved = set_fine_tuning_config(None, Some(&settings), &environment)
        .expect("the list sets")
        .expect("the list is configured");
    assert!(
        fine_tuning_config_for(Some(&resolved), "azure")
            .expect("the list is set")
            .is_none()
    );
}

#[test]
fn a_null_set_keeps_the_existing_configuration() {
    let environment = environment();
    let existing = set_fine_tuning_config(
        None,
        Some(&Json::List(vec![entry("azure", text("first"))])),
        &environment,
    )
    .expect("the list sets")
    .expect("the list is configured");
    let kept = set_fine_tuning_config(Some(existing.clone()), Some(&Json::Null), &environment)
        .expect("a null set keeps the list");
    assert_eq!(kept.as_ref(), Some(&existing));
    let kept =
        set_fine_tuning_config(kept, None, &environment).expect("an absent set keeps the list");
    assert_eq!(kept, Some(existing));
}

#[test]
fn a_non_list_set_is_rejected() {
    let environment = environment();
    let error = set_fine_tuning_config(None, Some(&text("azure")), &environment)
        .expect_err("a string is not a list");
    assert_eq!(
        error.to_string(),
        "invalid fine_tuning config, expected a list is not a list"
    );
    assert!(matches!(error, FineTuningConfigError::NotAList));
}

#[test]
fn a_lookup_before_set_names_the_config_file() {
    let error = fine_tuning_config_for(None, "azure").expect_err("nothing is configured");
    assert_eq!(
        error.to_string(),
        "fine_tuning_config is not set, set it on your config.yaml file."
    );
    assert!(matches!(error, FineTuningConfigError::Unset));
}

#[test]
fn a_scalar_finetune_settings_fails_the_load_naming_the_key() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("finetune_settings: azure\n")
    .expect_err("a scalar is not a list");
    assert!(
        matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "finetune_settings"),
        "{error:?}"
    );
    assert_eq!(error.category(), "invalid_scalar");
}

#[test]
fn a_map_finetune_settings_fails_the_load_naming_the_key() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("finetune_settings: {custom_llm_provider: azure}\n")
    .expect_err("a map is not a list");
    assert!(
        matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "finetune_settings"),
        "{error:?}"
    );
}

#[rstest]
#[case::boolean_true("finetune_settings: true\n", true)]
#[case::boolean_false("finetune_settings: false\n", false)]
#[case::text("finetune_settings: azure\n", false)]
fn a_scalar_finetune_settings_carries_the_written_boolean(
    #[case] source: &str,
    #[case] flag: bool,
) {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect_err("a scalar is not a list");
    assert!(
        matches!(&error, ConfigError::InvalidScalar { path, value } if path == "finetune_settings" && *value == flag),
        "{error:?}"
    );
}

#[test]
fn a_configured_finetune_settings_is_not_reported_unsupported() {
    let environment = environment();
    let config = ConfigLoader {
        lookup: environment,
    }
    .load(
        "finetune_settings:\n  - custom_llm_provider: azure\n    api_key: os.environ/AZURE_TEST\n",
    )
    .expect("the accepted key is not unsupported");
    assert!(
        config
            .sections
            .fine_tuning_provider_config
            .finetune_settings
            .is_some()
    );
}

#[test]
fn a_missing_secret_fails_the_load_naming_the_variable() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("finetune_settings:\n  - custom_llm_provider: azure\n    api_key: os.environ/AZURE_MISSING\n")
    .expect_err("the secret is not set");
    assert!(
        matches!(
            &error,
            ConfigError::MissingEnvironment { variable, path }
                if variable == "AZURE_MISSING" && path == "finetune_settings"
        ),
        "{error:?}"
    );
}
