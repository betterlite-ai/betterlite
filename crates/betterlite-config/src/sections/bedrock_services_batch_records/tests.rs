#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::bedrock_services_batch_records::Settings;

const ALIAS_CONFIG: &str = "
litellm_settings:
  model_alias_map:
    global-only: amazon.nova-pro-v1:0
    shared: anthropic.claude-3-haiku-20240307-v1:0
  callbacks: []
model_list:
  - model_name: first
    litellm_params:
      model: bedrock/first
      model_alias_map:
        deployment-only: openai.gpt-oss-20b-1:0
        shared: amazon.nova-lite-v1:0
  - model_name: second
    litellm_params:
      model: bedrock/second
";

#[rstest]
#[case::loads_model_alias_map(ALIAS_CONFIG)]
fn batch_records_config_cases(#[case] source: &str) {
    let settings = settings(source).expect("the alias configuration loads");
    assert_eq!(
        settings.model_alias_map,
        BTreeMap::from([
            (
                String::from("deployment-only"),
                String::from("openai.gpt-oss-20b-1:0")
            ),
            (
                String::from("global-only"),
                String::from("amazon.nova-pro-v1:0")
            ),
            (
                String::from("shared"),
                String::from("amazon.nova-lite-v1:0")
            ),
        ])
    );
    // the loader reads the same map the pre-call section keeps
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(ALIAS_CONFIG)
    .expect("the configuration loads");
    assert_eq!(
        config.sections.pre_call.model_alias_map.get("global-only"),
        Some(&String::from("amazon.nova-pro-v1:0"))
    );
    assert!(
        !config
            .sections
            .pre_call
            .model_alias_map
            .contains_key("deployment-only")
    );
}

#[test]
fn defaults_to_an_empty_map() {
    let settings = settings("model_list: []\n").expect("the defaults load");
    assert!(settings.model_alias_map.is_empty());
}

#[test]
fn unrelated_keys_still_load() {
    let settings =
        settings("litellm_settings:\n  drop_params: true\n").expect("unrelated keys still load");
    assert!(settings.model_alias_map.is_empty());
}

// pre_call's own null alias map test loads through every section, this one included
#[test]
fn null_alias_maps_load_as_empty() {
    let source = "
litellm_settings:
  model_alias_map: null
model_list:
  - model_name: first
    litellm_params:
      model: bedrock/first
      model_alias_map: null
";
    let settings = settings(source).expect("an explicit null alias map is empty");
    assert!(settings.model_alias_map.is_empty());
}

fn settings(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .map(|config| config.sections.bedrock_services_batch_records.clone())
}
