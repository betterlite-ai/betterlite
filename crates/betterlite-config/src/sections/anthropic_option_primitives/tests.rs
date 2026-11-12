#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;

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

fn metadata(value: &str) -> IndexMap<String, Json> {
    serde_json::from_str(value).unwrap()
}

fn assert_key<T>(key: &str, deployed: (&str, &T), global: (&str, &T), read: fn(&OptionValues) -> T)
where
    T: Debug + PartialEq,
{
    let defaults = load("", &[]).unwrap();
    assert_eq!(
        read(&defaults.litellm_settings),
        read(&OptionValues::default())
    );
    assert!(defaults.deployments.is_empty(), "{key}");
    let deployment_only = load(&deployment(key, deployed.0), &[]).unwrap();
    assert_eq!(read(&deployment_only.deployments[0]), *deployed.1, "{key}");
    let global_only = load(&settings(key, global.0), &[]).unwrap();
    assert_eq!(read(&global_only.litellm_settings), *global.1, "{key}");
    let both_source = format!("{}{}", deployment(key, deployed.0), settings(key, global.0));
    let both = load(&both_source, &[]).unwrap();
    assert_eq!(read(&both.deployments[0]), *deployed.1, "{key}");
    assert_eq!(read(&both.litellm_settings), *global.1, "{key}");
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
    let set = |value: &str| {
        load("", &[(variable, value)])
            .unwrap()
            .default_chat_max_tokens
    };
    assert_eq!(load("", &[]).unwrap().default_chat_max_tokens, 4096);
    assert_eq!(set("8192"), 8192);
    assert_eq!(set(" 2048 "), 2048);
    assert_eq!(set("-1"), -1);
    assert_eq!(set(""), 4096, "an empty value counts as unset");
    let error = load("", &[(variable, "not-a-number")]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("DEFAULT_ANTHROPIC_CHAT_MAX_TOKENS"),
        "{error}"
    );
}

fn max_tokens_case() {
    assert_key("max_tokens", ("7", &Some(7)), ("9", &Some(9)), |values| {
        values.max_tokens
    });
    for (raw, expected) in [("0", 0), ("os.environ/ANTHROPIC_MAX_TOKENS", 321)] {
        let source = deployment("max_tokens", raw);
        let loaded = load(&source, &[("ANTHROPIC_MAX_TOKENS", "321")]).unwrap();
        assert_eq!(loaded.deployments[0].max_tokens, Some(expected));
    }
}

fn stop_sequences_case() {
    let deployed = Some(vec!["END".to_owned(), "STOP".to_owned()]);
    let global = Some(vec!["GLOBAL".to_owned()]);
    assert_key(
        "stop_sequences",
        ("[END, STOP]", &deployed),
        ("[GLOBAL]", &global),
        |values| values.stop_sequences.clone(),
    );
    let empty = load(&deployment("stop_sequences", "[]"), &[]).unwrap();
    assert_eq!(empty.deployments[0].stop_sequences, Some(Vec::new()));
}

fn temperature_case() {
    assert_key(
        "temperature",
        ("0.5", &Some(0.5_f64)),
        ("0.25", &Some(0.25_f64)),
        |values| values.temperature,
    );
    let reference = load(
        &deployment("temperature", "os.environ/ANTHROPIC_TEMPERATURE"),
        &[("ANTHROPIC_TEMPERATURE", "0.75")],
    )
    .unwrap();
    assert_eq!(reference.deployments[0].temperature, Some(0.75_f64));
    let zero = load(&deployment("temperature", "0"), &[]).unwrap();
    assert_eq!(zero.deployments[0].temperature, Some(0.0_f64));
}

fn top_p_case() {
    assert_key(
        "top_p",
        ("0.9", &Some(0.9_f64)),
        ("0.1", &Some(0.1_f64)),
        |values| values.top_p,
    );
}

fn top_k_case() {
    assert_key("top_k", ("40", &Some(40)), ("20", &Some(20)), |values| {
        values.top_k
    });
}

fn metadata_case() {
    assert_key(
        "metadata",
        (
            "{user_id: deployed}",
            &Some(metadata(r#"{"user_id": "deployed"}"#)),
        ),
        (
            "{user_id: global}",
            &Some(metadata(r#"{"user_id": "global"}"#)),
        ),
        |values| values.metadata.clone(),
    );
    assert_eq!(
        load(&deployment("metadata", "{}"), &[])
            .unwrap()
            .deployments[0]
            .metadata,
        Some(IndexMap::new())
    );
    let reference = load(
        &deployment(
            "metadata",
            "{user_id: os.environ/ANTHROPIC_USER, nested: {note: os.environ/ANTHROPIC_NOTE}, tags: [os.environ/ANTHROPIC_TAG]}",
        ),
        &[
            ("ANTHROPIC_USER", "from-env"),
            ("ANTHROPIC_NOTE", "hi"),
            ("ANTHROPIC_TAG", "a"),
        ],
    )
    .unwrap();
    assert_eq!(
        reference.deployments[0].metadata,
        Some(metadata(
            r#"{"user_id": "from-env", "nested": {"note": "hi"}, "tags": ["a"]}"#
        ))
    );
}

fn system_case() {
    let deployed = Some("be deployed".to_owned());
    let global = Some("be global".to_owned());
    assert_key(
        "system",
        ("\"be deployed\"", &deployed),
        ("\"be global\"", &global),
        |values| values.system.clone(),
    );
    assert_eq!(
        load(&deployment("system", "\"\""), &[])
            .unwrap()
            .deployments[0]
            .system,
        Some(String::new())
    );
    let reference = load(
        &deployment("system", "os.environ/ANTHROPIC_SYSTEM"),
        &[("ANTHROPIC_SYSTEM", "be from env")],
    )
    .unwrap();
    assert_eq!(
        reference.deployments[0].system,
        Some("be from env".to_owned())
    );
    let error = load(&deployment("system", "os.environ/ANTHROPIC_MISSING"), &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, .. } if variable == "ANTHROPIC_MISSING"),
        "{error}"
    );
}

fn unrelated_key_case() {
    let error = load(&deployment("max_tokens_extra", "1"), &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.max_tokens_extra"]),
        "{error}"
    );
    let global_error = load("litellm_settings: {max_tokens_extra: 1}\n", &[]).unwrap_err();
    assert!(
        matches!(&global_error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.max_tokens_extra"]),
        "{global_error}"
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
        temperature: Some(0.4_f64),
        top_p: Some(0.8_f64),
        top_k: Some(40),
        metadata: Some(metadata(r#"{"user_id": "seed"}"#)),
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
