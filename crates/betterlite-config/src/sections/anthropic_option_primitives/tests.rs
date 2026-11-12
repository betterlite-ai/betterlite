#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{OptionValues, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_option_primitives)
}

fn deployment(key: &str, value: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: anthropic/unit, {key}: {value}}}\n"
    )
}

fn settings(key: &str, value: &str) -> String {
    format!("litellm_settings: {{{key}: {value}}}\n")
}

fn metadata(value: &str) -> Option<IndexMap<String, Json>> {
    Some(serde_json::from_str(value).unwrap())
}

fn max_tokens(value: Option<i64>) -> OptionValues {
    OptionValues {
        max_tokens: value,
        stop_sequences: None,
        temperature: None,
        top_p: None,
        top_k: None,
        metadata: None,
        system: None,
    }
}

fn stop_sequences(value: Option<Vec<String>>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: value,
        temperature: None,
        top_p: None,
        top_k: None,
        metadata: None,
        system: None,
    }
}

fn temperature(value: Option<f64>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: None,
        temperature: value,
        top_p: None,
        top_k: None,
        metadata: None,
        system: None,
    }
}

fn top_p(value: Option<f64>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: None,
        temperature: None,
        top_p: value,
        top_k: None,
        metadata: None,
        system: None,
    }
}

fn top_k(value: Option<i64>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: None,
        temperature: None,
        top_p: None,
        top_k: value,
        metadata: None,
        system: None,
    }
}

fn metadata_value(value: Option<IndexMap<String, Json>>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: None,
        temperature: None,
        top_p: None,
        top_k: None,
        metadata: value,
        system: None,
    }
}

fn system(value: Option<String>) -> OptionValues {
    OptionValues {
        max_tokens: None,
        stop_sequences: None,
        temperature: None,
        top_p: None,
        top_k: None,
        metadata: None,
        system: value,
    }
}

fn assert_value(key: &str, deployed: &str, global: &str, expects: &OptionValues) {
    let defaults = load("", &[]).unwrap();
    assert_eq!(defaults.litellm_settings, OptionValues::default(), "{key}");
    assert!(defaults.deployments.is_empty(), "{key}");
    let deployment_only = load(&deployment(key, deployed), &[]).unwrap();
    assert_eq!(deployment_only.deployments, vec![expects.clone()], "{key}");
    let global_only = load(&settings(key, global), &[]).unwrap();
    assert_eq!(global_only.litellm_settings, expects.clone(), "{key}");
    let both = format!("{}{}", deployment(key, deployed), settings(key, global));
    let both = load(&both, &[]).unwrap();
    assert_eq!(both.deployments, vec![expects.clone()], "{key}");
    assert_eq!(both.litellm_settings, expects.clone(), "{key}");
}

#[derive(Clone, Copy, Debug)]
enum Case {
    DefaultChatMaxTokens,
    MaxTokens,
    StopSequences,
    Temperature,
    TopP,
    TopK,
    Metadata,
    System,
    UnrelatedKey,
}

#[rstest]
#[case::loads_default_anthropic_chat_max_tokens(Case::DefaultChatMaxTokens)]
#[case::loads_anthropicconfig_max_tokens(Case::MaxTokens)]
#[case::loads_anthropicconfig_stop_sequences(Case::StopSequences)]
#[case::loads_anthropicconfig_temperature(Case::Temperature)]
#[case::loads_anthropicconfig_top_p(Case::TopP)]
#[case::loads_anthropicconfig_top_k(Case::TopK)]
#[case::loads_anthropicconfig_metadata(Case::Metadata)]
#[case::loads_anthropicconfig_system(Case::System)]
#[case::rejects_unrelated_key(Case::UnrelatedKey)]
fn anthropic_defaults_config_cases(#[case] case: Case) {
    match case {
        Case::DefaultChatMaxTokens => default_chat_max_tokens_case(),
        Case::MaxTokens => max_tokens_case(),
        Case::StopSequences => stop_sequences_case(),
        Case::Temperature => temperature_case(),
        Case::TopP => top_p_case(),
        Case::TopK => top_k_case(),
        Case::Metadata => metadata_case(),
        Case::System => system_case(),
        Case::UnrelatedKey => unrelated_key_case(),
    }
}

fn default_chat_max_tokens_case() {
    let variable = "DEFAULT_ANTHROPIC_CHAT_MAX_TOKENS";
    assert_eq!(load("", &[]).unwrap().default_chat_max_tokens, 4096);
    let set = |value: &str| {
        load("", &[(variable, value)])
            .unwrap()
            .default_chat_max_tokens
    };
    assert_eq!(set("8192"), 8192);
    assert_eq!(set(" 2048 "), 2048);
    assert_eq!(set("-1"), -1);
    assert_eq!(set(""), 4096, "an empty value counts as unset");
    assert_eq!(set("not-a-number"), 4096);
}

fn max_tokens_case() {
    assert_value("max_tokens", "7", "9", &max_tokens(Some(7)));
    let zero = load(&deployment("max_tokens", "0"), &[]).unwrap();
    assert_eq!(zero.deployments, vec![max_tokens(Some(0))]);
    let reference = load(
        &deployment("max_tokens", "os.environ/ANTHROPIC_MAX_TOKENS"),
        &[("ANTHROPIC_MAX_TOKENS", "321")],
    )
    .unwrap();
    assert_eq!(reference.deployments, vec![max_tokens(Some(321))]);
}

fn stop_sequences_case() {
    assert_value(
        "stop_sequences",
        "[END, STOP]",
        "[GLOBAL]",
        &stop_sequences(Some(vec!["END".to_owned(), "STOP".to_owned()])),
    );
    let empty = load(&deployment("stop_sequences", "[]"), &[]).unwrap();
    assert_eq!(empty.deployments, vec![stop_sequences(Some(Vec::new()))]);
}

fn temperature_case() {
    assert_value("temperature", "0.5", "0.25", &temperature(Some(0.5)));
    let zero = load(&deployment("temperature", "0"), &[]).unwrap();
    assert_eq!(zero.deployments, vec![temperature(Some(0.0))]);
    let reference = load(
        &deployment("temperature", "os.environ/ANTHROPIC_TEMPERATURE"),
        &[("ANTHROPIC_TEMPERATURE", "0.75")],
    )
    .unwrap();
    assert_eq!(reference.deployments, vec![temperature(Some(0.75))]);
}

fn top_p_case() {
    assert_value("top_p", "0.9", "0.1", &top_p(Some(0.9)));
}

fn top_k_case() {
    assert_value("top_k", "40", "20", &top_k(Some(40)));
}

fn metadata_case() {
    assert_value(
        "metadata",
        "{user_id: deployed}",
        "{user_id: global}",
        &metadata_value(metadata(r#"{"user_id": "deployed"}"#)),
    );
    let empty = load(&deployment("metadata", "{}"), &[]).unwrap();
    assert_eq!(
        empty.deployments,
        vec![metadata_value(Some(IndexMap::new()))]
    );
}

fn system_case() {
    assert_value(
        "system",
        "\"be deployed\"",
        "\"be global\"",
        &system(Some("be deployed".to_owned())),
    );
    let empty = load(&deployment("system", "\"\""), &[]).unwrap();
    assert_eq!(empty.deployments, vec![system(Some(String::new()))]);
}

fn unrelated_key_case() {
    let error = load(&deployment("max_tokens_extra", "1"), &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.max_tokens_extra"]),
        "{error}"
    );
    let error = load("litellm_settings: {max_tokens_extra: 1}\n", &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.max_tokens_extra"]),
        "{error}"
    );
}

#[test]
fn loads_the_anthropic_defaults_corpus_seeds() {
    let valid = include_str!("../../../fuzz/corpus/config_yaml/anthropic_defaults-valid.yaml");
    let loaded = load(valid, &[]).unwrap();
    assert_eq!(loaded.default_chat_max_tokens, 4096);
    let seed = OptionValues {
        max_tokens: Some(8192),
        stop_sequences: Some(vec!["END".to_owned()]),
        temperature: Some(0.4),
        top_p: Some(0.8),
        top_k: Some(40),
        metadata: metadata(r#"{"user_id": "seed"}"#),
        system: Some("be brief".to_owned()),
    };
    assert_eq!(loaded.deployments, vec![seed]);
    let unrelated =
        include_str!("../../../fuzz/corpus/config_yaml/anthropic_defaults-unrelated.yaml");
    let error = load(unrelated, &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.anthropic_option_extra"]),
        "{error}"
    );
}
