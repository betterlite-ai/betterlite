#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::{OptionValues, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn loaded(source: &str, environment: &[(&str, &str)]) -> Result<crate::types::Config, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(source)
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    Ok(loaded(source, environment)?.sections.anthropic_completion)
}

fn deployment(key: &str, value: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: anthropic_text/claude-2, {key}: {value}}}\n"
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
    MaxTokensToSample,
    StopSequences,
    Temperature,
    TopP,
    TopK,
    Metadata,
    UnrelatedKey,
}

#[rstest]
#[case::loads_anthropictextconfig_max_tokens_to_sample(Case::MaxTokensToSample)]
#[case::loads_anthropictextconfig_stop_sequences(Case::StopSequences)]
#[case::loads_anthropictextconfig_temperature(Case::Temperature)]
#[case::loads_anthropictextconfig_top_p(Case::TopP)]
#[case::loads_anthropictextconfig_top_k(Case::TopK)]
#[case::loads_anthropictextconfig_metadata(Case::Metadata)]
#[case::rejects_unrelated_key(Case::UnrelatedKey)]
fn anthropic_legacy_config_cases(#[case] case: Case) {
    match case {
        Case::MaxTokensToSample => max_tokens_to_sample_case(),
        Case::StopSequences => stop_sequences_case(),
        Case::Temperature => temperature_case(),
        Case::TopP => top_p_case(),
        Case::TopK => top_k_case(),
        Case::Metadata => metadata_case(),
        Case::UnrelatedKey => unrelated_key_case(),
    }
}

fn max_tokens_to_sample_case() {
    assert_key(
        "max_tokens_to_sample",
        ("7", &Some(7)),
        ("9", &Some(9)),
        |values| values.max_tokens_to_sample,
    );
    for (raw, expected) in [("0", 0), ("os.environ/ANTHROPIC_SAMPLE", 321)] {
        let source = deployment("max_tokens_to_sample", raw);
        let config = loaded(&source, &[("ANTHROPIC_SAMPLE", "321")]).unwrap();
        assert_eq!(
            config.sections.anthropic_completion.deployments[0].max_tokens_to_sample,
            Some(expected)
        );
    }
    // the constructor default is DEFAULT_MAX_TOKENS, which the core loader owns
    assert_eq!(
        loaded("", &[]).unwrap().litellm_settings.default_max_tokens,
        4096
    );
    let overridden = loaded("", &[("DEFAULT_MAX_TOKENS", "2048")]).unwrap();
    assert_eq!(overridden.litellm_settings.default_max_tokens, 2048);
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
    let empty = load(&deployment("metadata", "{}"), &[]).unwrap();
    assert_eq!(empty.deployments[0].metadata, Some(IndexMap::new()));
    let reference = load(
        &deployment(
            "metadata",
            "{user_id: os.environ/LEGACY_USER, nested: {note: os.environ/LEGACY_NOTE}, tags: [os.environ/LEGACY_ABSENT], entries: [{note: os.environ/LEGACY_ENTRY}]}",
        ),
        &[
            ("LEGACY_USER", "from-env"),
            ("LEGACY_NOTE", "hi"),
            ("LEGACY_ENTRY", "entry"),
        ],
    )
    .unwrap();
    assert_eq!(
        reference.deployments[0].metadata,
        Some(metadata(
            r#"{"user_id": "from-env", "nested": {"note": "hi"}, "tags": ["os.environ/LEGACY_ABSENT"], "entries": [{"note": "entry"}]}"#
        ))
    );
    let error = load(
        &deployment("metadata", "{user_id: os.environ/LEGACY_MISSING}"),
        &[],
    )
    .unwrap_err();
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable, .. } if variable == "LEGACY_MISSING"),
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
fn loads_the_anthropic_legacy_corpus_seeds() {
    let valid = include_str!("../../../fuzz/corpus/config_yaml/anthropic_legacy-valid.yaml");
    let config = load(valid, &[]).unwrap();
    let seed = OptionValues {
        max_tokens_to_sample: Some(512),
        stop_sequences: Some(vec!["END".to_owned()]),
        temperature: Some(0.4_f64),
        top_p: Some(0.8_f64),
        top_k: Some(40),
        metadata: Some(metadata(r#"{"user_id": "seed"}"#)),
    };
    assert_eq!(config.deployments, vec![seed]);
    assert_eq!(config.litellm_settings.max_tokens_to_sample, Some(1024));
    let unrelated =
        include_str!("../../../fuzz/corpus/config_yaml/anthropic_legacy-unrelated.yaml");
    let error = load(unrelated, &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.anthropic_legacy_extra"]),
        "{error}"
    );
}
