use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use crate::api::rerank_provider_plan::{Environment, loads_rerank_settings};
use crate::loader::ConfigError;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn text(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[test]
fn unset_settings_take_their_defaults() {
    let settings = loads_rerank_settings("litellm_settings: {}\n", &environment(&[])).unwrap();
    assert_eq!(text(settings.api_key.as_ref()), None);
    assert_eq!(settings.api_base, None);
    assert_eq!(settings.headers, None);
    assert_eq!(text(settings.togetherai_api_key.as_ref()), None);
    assert!(!settings.drop_params);
    assert_default_environment_bases(&settings.environment);
    let environment = &settings.environment;
    assert_eq!(
        [
            environment.togetherai_api_key.is_none(),
            environment.jina_ai_api_key.is_none(),
            environment.nvidia_nim_api_key.is_none(),
            environment.hosted_vllm_api_key.is_none(),
            environment.deepinfra_api_key.is_none(),
            environment.fireworks_api_key.is_none(),
            environment.fireworks_ai_api_key.is_none(),
            environment.fireworksai_api_key.is_none(),
            environment.fireworks_ai_token.is_none(),
            environment.voyage_api_key.is_none(),
            environment.voyage_ai_api_key.is_none(),
        ],
        [true; 11]
    );
}

fn assert_default_environment_bases(environment: &Environment) {
    assert_eq!(environment.cohere_api_base, None);
    assert_eq!(environment.azure_ai_api_base, None);
    assert_eq!(environment.infinity_api_base, None);
    assert_eq!(environment.bedrock_api_base, None);
    assert_eq!(environment.nvidia_nim_api_base, None);
    assert_eq!(environment.hosted_vllm_api_base, None);
    assert_eq!(environment.deepinfra_api_base, None);
    assert_eq!(environment.fireworks_ai_api_base, None);
    assert_eq!(environment.voyage_api_base, None);
}

#[test]
fn explicit_settings_are_read() {
    let settings = loads_rerank_settings(
        "litellm_settings:\n  api_key: sk-global\n  api_base: https://global.test/v1\n  headers: {x-global: a}\n  togetherai_api_key: together-global\n  drop_params: true\n",
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(text(settings.api_key.as_ref()), Some("sk-global"));
    assert_eq!(settings.api_base.as_deref(), Some("https://global.test/v1"));
    assert_eq!(
        settings.headers,
        Some(BTreeMap::from([("x-global".to_owned(), "a".to_owned())]))
    );
    assert_eq!(
        text(settings.togetherai_api_key.as_ref()),
        Some("together-global")
    );
    assert!(settings.drop_params);
}

#[test]
fn environment_references_resolve() {
    let settings = loads_rerank_settings(
        "litellm_settings:\n  api_key: os.environ/GLOBAL_KEY\n  api_base: os.environ/GLOBAL_BASE\n  headers: {x-global: os.environ/GLOBAL_HEADER}\n  togetherai_api_key: os.environ/TOGETHER_KEY\n",
        &environment(&[
            ("GLOBAL_KEY", "sk-resolved"),
            ("GLOBAL_BASE", "https://resolved.test"),
            ("GLOBAL_HEADER", "header-value"),
            ("TOGETHER_KEY", "together-resolved"),
        ]),
    )
    .unwrap();
    assert_eq!(text(settings.api_key.as_ref()), Some("sk-resolved"));
    assert_eq!(settings.api_base.as_deref(), Some("https://resolved.test"));
    assert_eq!(
        settings.headers,
        Some(BTreeMap::from([(
            "x-global".to_owned(),
            "header-value".to_owned()
        )]))
    );
    assert_eq!(
        text(settings.togetherai_api_key.as_ref()),
        Some("together-resolved")
    );
}

#[test]
fn a_missing_environment_reference_names_its_path() {
    let error = loads_rerank_settings(
        "litellm_settings:\n  api_key: os.environ/ABSENT\n",
        &environment(&[]),
    )
    .unwrap_err();
    let ConfigError::MissingEnvironment { variable, path } = error else {
        panic!("the missing variable is reported");
    };
    assert_eq!(variable, "ABSENT");
    assert_eq!(path, "litellm_settings.api_key");
}

#[test]
fn empty_values_count_as_unset() {
    let settings = loads_rerank_settings(
        "litellm_settings:\n  api_key: \"\"\n  api_base: \"\"\n  headers: {}\n  togetherai_api_key: \"\"\n",
        &environment(&[]),
    )
    .unwrap();
    assert_eq!(text(settings.api_key.as_ref()), None);
    assert_eq!(settings.api_base, None);
    assert_eq!(settings.headers, None);
    assert_eq!(text(settings.togetherai_api_key.as_ref()), None);
}

#[test]
fn the_drop_flag_reads_the_environment() {
    let enabled = loads_rerank_settings(
        "model_list: []\n",
        &environment(&[("LITELLM_DROP_PARAMS", "true")]),
    )
    .unwrap();
    assert!(enabled.drop_params);
    let disabled = loads_rerank_settings(
        "model_list: []\n",
        &environment(&[("LITELLM_DROP_PARAMS", "false")]),
    )
    .unwrap();
    assert!(!disabled.drop_params);
    let empty = loads_rerank_settings(
        "model_list: []\n",
        &environment(&[("LITELLM_DROP_PARAMS", "")]),
    )
    .unwrap();
    assert!(!empty.drop_params);
}

#[test]
fn an_invalid_drop_flag_turns_it_on() {
    let settings = loads_rerank_settings(
        "model_list: []\n",
        &environment(&[("LITELLM_DROP_PARAMS", "sometimes")]),
    )
    .unwrap();
    assert!(settings.drop_params);
}

#[test]
fn the_yaml_drop_flag_wins_over_the_environment() {
    let settings = loads_rerank_settings(
        "litellm_settings:\n  drop_params: false\n",
        &environment(&[("LITELLM_DROP_PARAMS", "true")]),
    )
    .unwrap();
    assert!(!settings.drop_params);
}

const PROVIDER_VALUES: [(&str, &str); 20] = [
    ("COHERE_API_BASE", "https://cohere.test"),
    ("AZURE_AI_API_BASE", "https://azure.test"),
    ("INFINITY_API_BASE", "https://infinity.test"),
    ("BEDROCK_API_BASE", "https://bedrock.test"),
    ("TOGETHERAI_API_KEY", "together-key"),
    ("JINA_AI_API_KEY", "jina-key"),
    ("NVIDIA_NIM_API_KEY", "nvidia-key"),
    ("NVIDIA_NIM_API_BASE", "https://nvidia.test"),
    ("HOSTED_VLLM_API_KEY", "vllm-key"),
    ("HOSTED_VLLM_API_BASE", "https://vllm.test"),
    ("DEEPINFRA_API_KEY", "deepinfra-key"),
    ("DEEPINFRA_API_BASE", "https://deepinfra.test"),
    ("FIREWORKS_API_KEY", "fireworks-key"),
    ("FIREWORKS_AI_API_KEY", "fireworks-ai-key"),
    ("FIREWORKSAI_API_KEY", "fireworksai-key"),
    ("FIREWORKS_AI_TOKEN", "fireworks-token"),
    ("FIREWORKS_AI_API_BASE", "https://fireworks.test"),
    ("VOYAGE_API_KEY", "voyage-key"),
    ("VOYAGE_AI_API_KEY", "voyage-ai-key"),
    ("VOYAGE_API_BASE", "https://voyage.test"),
];

#[test]
fn provider_environment_values_are_read() {
    let settings =
        loads_rerank_settings("model_list: []\n", &environment(&PROVIDER_VALUES)).unwrap();
    assert_base_values(&settings.environment);
    assert_secret_values(&settings.environment);
}

fn assert_base_values(environment: &Environment) {
    assert_eq!(
        environment.cohere_api_base.as_deref(),
        Some("https://cohere.test")
    );
    assert_eq!(
        environment.azure_ai_api_base.as_deref(),
        Some("https://azure.test")
    );
    assert_eq!(
        environment.infinity_api_base.as_deref(),
        Some("https://infinity.test")
    );
    assert_eq!(
        environment.bedrock_api_base.as_deref(),
        Some("https://bedrock.test")
    );
    assert_eq!(
        environment.nvidia_nim_api_base.as_deref(),
        Some("https://nvidia.test")
    );
    assert_eq!(
        environment.hosted_vllm_api_base.as_deref(),
        Some("https://vllm.test")
    );
    assert_eq!(
        environment.deepinfra_api_base.as_deref(),
        Some("https://deepinfra.test")
    );
    assert_eq!(
        environment.fireworks_ai_api_base.as_deref(),
        Some("https://fireworks.test")
    );
    assert_eq!(
        environment.voyage_api_base.as_deref(),
        Some("https://voyage.test")
    );
}

fn assert_secret_values(environment: &Environment) {
    let values = [
        text(environment.togetherai_api_key.as_ref()),
        text(environment.jina_ai_api_key.as_ref()),
        text(environment.nvidia_nim_api_key.as_ref()),
        text(environment.hosted_vllm_api_key.as_ref()),
        text(environment.deepinfra_api_key.as_ref()),
        text(environment.fireworks_api_key.as_ref()),
        text(environment.fireworks_ai_api_key.as_ref()),
        text(environment.fireworksai_api_key.as_ref()),
        text(environment.fireworks_ai_token.as_ref()),
        text(environment.voyage_api_key.as_ref()),
        text(environment.voyage_ai_api_key.as_ref()),
    ];
    assert_eq!(
        values,
        [
            Some("together-key"),
            Some("jina-key"),
            Some("nvidia-key"),
            Some("vllm-key"),
            Some("deepinfra-key"),
            Some("fireworks-key"),
            Some("fireworks-ai-key"),
            Some("fireworksai-key"),
            Some("fireworks-token"),
            Some("voyage-key"),
            Some("voyage-ai-key"),
        ]
    );
}

#[test]
fn an_empty_provider_environment_value_counts_as_unset() {
    let settings = loads_rerank_settings(
        "model_list: []\n",
        &environment(&[("COHERE_API_BASE", ""), ("JINA_AI_API_KEY", "")]),
    )
    .unwrap();
    assert_eq!(settings.environment.cohere_api_base, None);
    assert_eq!(text(settings.environment.jina_ai_api_key.as_ref()), None);
}
