#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::prompts::Settings;

fn loaded(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.prompts)
}

fn settings(source: &str) -> Settings {
    loaded(source).expect("the prompts settings load")
}

#[test]
fn an_absent_prompts_key_reads_as_an_empty_list() {
    assert!(settings("model_list: []\n").prompts.is_empty());
}

#[test]
fn a_null_prompts_key_reads_as_an_empty_list() {
    assert!(settings("prompts:\n").prompts.is_empty());
}

#[test]
fn an_empty_prompts_list_reads_as_an_empty_list() {
    assert!(settings("prompts: []\n").prompts.is_empty());
}

#[test]
fn entries_keep_their_order_and_their_raw_fields() {
    let settings = settings(
        "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n      dotprompt_content: Hello\n    future_option: true\n  - prompt_id: farewell\n    litellm_params:\n      prompt_integration: gitlab\n",
    );
    assert_eq!(settings.prompts.len(), 2);
    let expected: IndexMap<String, Json> = serde_json::from_str(
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"Hello"},"future_option":true}"#,
    )
    .expect("the expected entry is a mapping");
    assert_eq!(settings.prompts[0], expected);
    assert_eq!(
        settings.prompts[0]
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["prompt_id", "litellm_params", "future_option"]
    );
    assert_eq!(
        settings.prompts[1].get("prompt_id"),
        Some(&Json::Text("farewell".to_owned()))
    );
}

#[rstest]
#[case::text("prompts: not-a-list\n")]
#[case::mapping("prompts:\n  prompt_id: greeting\n")]
#[case::number("prompts: 5\n")]
fn a_prompts_value_that_is_not_a_list_is_refused_and_names_the_key(#[case] source: &str) {
    let error = loaded(source).expect_err("the prompts value is not a list");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts"));
    assert!(error.to_string().contains("prompts"), "{error}");
}

#[rstest]
#[case::text("prompts:\n  - not-a-mapping\n")]
#[case::number("prompts:\n  - 5\n")]
#[case::list("prompts:\n  - [a, b]\n")]
#[case::null("prompts:\n  - null\n")]
fn an_entry_that_is_not_a_mapping_is_refused_and_names_its_index(#[case] source: &str) {
    let error = loaded(source).expect_err("the entry is not a mapping");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts[0]"));
    assert!(error.to_string().contains("prompts[0]"), "{error}");
}

#[test]
fn a_bad_entry_is_named_by_its_position() {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n  - not-a-mapping\n";
    let error = loaded(source).expect_err("the second entry is not a mapping");
    assert!(matches!(&error, ConfigError::InvalidScalar { path, .. } if path == "prompts[1]"));
}
