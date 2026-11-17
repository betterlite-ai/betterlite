#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::families::{Penalty, Resolved};
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str) -> Result<Config, ConfigError> {
    load_with(source, &[])
}

fn load_with(source: &str, environment: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(source)
}

fn resolved(config: &Config, index: usize) -> Resolved {
    config
        .sections
        .bedrock_legacy_models
        .resolve(index, config.litellm_settings.default_max_tokens)
}

fn resolved_from(source: &str) -> Resolved {
    resolved(&load(source).unwrap(), 0)
}

fn deployment(fields: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, {fields}}}\n"
    )
}

fn global(fields: &str) -> String {
    format!("litellm_settings: {{{fields}}}\n")
}

fn both(key: &str, deployment_value: &str, global_value: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, {key}: {deployment_value}}}\nlitellm_settings: {{{key}: {global_value}}}\n"
    )
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn penalty(scale: i64) -> Penalty {
    IndexMap::from([("scale".to_owned(), Json::Number(Number::from(scale)))])
}

type Read<T> = fn(&Resolved) -> Option<T>;

fn assert_resolved<T>(
    key: &str,
    deployment_value: &str,
    global_value: &str,
    expected: (T, T),
    read: Read<T>,
) where
    T: Clone + Debug + PartialEq,
{
    let (deployment_expected, global_expected) = expected;
    let default = resolved_from("");
    assert_eq!(read(&default), None, "{key} default");
    let only_deployment = resolved_from(&deployment(&format!("{key}: {deployment_value}")));
    assert_eq!(
        read(&only_deployment),
        Some(deployment_expected.clone()),
        "{key} deployment"
    );
    let only_global = resolved_from(&global(&format!("{key}: {global_value}")));
    assert_eq!(
        read(&only_global),
        Some(global_expected.clone()),
        "{key} global"
    );
    let configured = load(&both(key, deployment_value, global_value)).unwrap();
    assert_eq!(
        read(&resolved(&configured, 0)),
        Some(deployment_expected),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        read(&resolved(&configured, 1)),
        Some(global_expected),
        "{key} without a deployment"
    );
}

fn assert_i64(key: &str, deployment_value: i64, global_value: i64, read: Read<i64>) {
    let text = (deployment_value.to_string(), global_value.to_string());
    assert_resolved(
        key,
        &text.0,
        &text.1,
        (deployment_value, global_value),
        read,
    );
}

fn assert_number(key: &str, deployment_value: &str, global_value: &str, read: Read<Number>) {
    let expected = (
        deployment_value.parse().unwrap(),
        global_value.parse().unwrap(),
    );
    assert_resolved(key, deployment_value, global_value, expected, read);
}

fn assert_text(key: &str, deployment_value: &str, global_value: &str, read: Read<String>) {
    let expected = (deployment_value.to_owned(), global_value.to_owned());
    assert_resolved(key, deployment_value, global_value, expected, read);
}

fn assert_list(
    key: &str,
    deployment_items: &[&str],
    global_items: &[&str],
    read: Read<Vec<String>>,
) {
    let text = (
        format!("[{}]", deployment_items.join(", ")),
        format!("[{}]", global_items.join(", ")),
    );
    let expected = (words(deployment_items), words(global_items));
    assert_resolved(key, &text.0, &text.1, expected, read);
    let empty = resolved_from(&deployment(&format!("{key}: []")));
    assert_eq!(read(&empty), Some(Vec::new()), "{key} empty");
}

fn assert_penalty(key: &str, read: Read<Penalty>) {
    let expected = (penalty(1), penalty(2));
    assert_resolved(key, "{scale: 1}", "{scale: 2}", expected, read);
}

fn resolved_with_reference(key: &str, variable: &str, value: &str) -> Resolved {
    let config = load_with(
        &deployment(&format!("{key}: os.environ/{variable}")),
        &[(variable, value)],
    )
    .unwrap();
    resolved(&config, 0)
}

fn path_of(key: &str) -> String {
    format!("model_list.0.litellm_params.{key}")
}

fn missing(key: &str, variable: &str) -> ConfigError {
    load(&deployment(&format!("{key}: os.environ/{variable}"))).unwrap_err()
}

fn invalid(key: &str, value: &str) -> ConfigError {
    load(&deployment(&format!("{key}: '{value}'"))).unwrap_err()
}

fn assert_missing_environment(key: &str, variable: &str) {
    let error = missing(key, variable);
    let path = path_of(key);
    assert!(
        matches!(&error, ConfigError::MissingEnvironment { variable: name, path: reported } if name == variable && *reported == path),
        "{error}"
    );
}

enum Invalid {
    Integer,
    Float,
    Scalar,
}

fn assert_invalid(key: &str, value: &str, kind: Invalid) {
    let error = invalid(key, value);
    let path = path_of(key);
    let matched = match kind {
        Invalid::Integer => matches!(
            &error,
            ConfigError::InvalidInteger { path: reported, .. } if *reported == path
        ),
        Invalid::Float => matches!(
            &error,
            ConfigError::InvalidFloat { path: reported, .. } if *reported == path
        ),
        Invalid::Scalar => matches!(
            &error,
            ConfigError::InvalidScalar { path: reported, value: false } if *reported == path
        ),
    };
    assert!(matched, "{error}");
}

fn ai21_max_tokens() {
    assert_i64("maxTokens", 16, 32, |values| values.ai21.max_tokens);
    let zero = resolved_from(&deployment("maxTokens: 0"));
    assert_eq!(
        zero.ai21.max_tokens,
        Some(0),
        "an explicit zero is used as given"
    );
    let referenced = resolved_with_reference("maxTokens", "AI21_MAX_TOKENS", "64");
    assert_eq!(
        referenced.ai21.max_tokens,
        Some(64),
        "an os.environ reference reads into the field"
    );
    assert_missing_environment("maxTokens", "ABSENT_AI21_MAX_TOKENS");
    assert_invalid("maxTokens", "many", Invalid::Integer);
}

fn ai21_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.ai21.temperature.clone()
    });
    let zero = resolved_from(&deployment("temperature: 0"));
    assert_eq!(
        zero.ai21.temperature.as_ref().map(Number::to_string),
        Some("0".to_owned()),
        "an explicit zero is used as given"
    );
    let quoted = resolved_from(&deployment("temperature: '0.75'"));
    assert_eq!(
        quoted.ai21.temperature.as_ref().map(Number::to_string),
        Some("0.75".to_owned()),
        "a quoted decimal reads as a number"
    );
    assert_invalid("temperature", "abc", Invalid::Float);
    assert_invalid("temperature", "1.", Invalid::Scalar);
}

fn ai21_top_p() {
    assert_number("topP", "0.9", "1", |values| values.ai21.top_p.clone());
    let referenced = resolved_with_reference("topP", "AI21_TOP_P", "0.6");
    assert_eq!(
        referenced.ai21.top_p.as_ref().map(Number::to_string),
        Some("0.6".to_owned()),
        "an os.environ reference reads into the field"
    );
}

fn ai21_stop_sequences() {
    assert_list("stopSequences", &["alpha", "beta"], &["gamma"], |values| {
        values.ai21.stop_sequences.clone()
    });
}

fn ai21_frequence_penalty() {
    assert_penalty("frequencePenalty", |values| {
        values.ai21.frequence_penalty.clone()
    });
}

fn ai21_presence_penalty() {
    assert_penalty("presencePenalty", |values| {
        values.ai21.presence_penalty.clone()
    });
}

fn ai21_count_penalty() {
    assert_penalty("countPenalty", |values| values.ai21.count_penalty.clone());
}

fn cohere_max_tokens() {
    assert_i64("max_tokens", 8, 16, |values| values.cohere.max_tokens);
    let referenced = resolved_with_reference("max_tokens", "COHERE_MAX_TOKENS", "24");
    assert_eq!(
        referenced.cohere.max_tokens,
        Some(24),
        "an os.environ reference reads into the field"
    );
}

fn cohere_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.cohere.temperature.clone()
    });
}

fn cohere_return_likelihood() {
    assert_text("return_likelihood", "GENERATION", "ALL", |values| {
        values.cohere.return_likelihood.clone()
    });
    let referenced =
        resolved_with_reference("return_likelihood", "COHERE_LIKELIHOOD", "GENERATION");
    assert_eq!(
        referenced.cohere.return_likelihood.as_deref(),
        Some("GENERATION"),
        "an os.environ reference reads into the field"
    );
    assert_missing_environment("return_likelihood", "ABSENT_COHERE_LIKELIHOOD");
}

fn llama_max_token_count() {
    assert_i64("maxTokenCount", 32, 64, |values| {
        values.llama.max_token_count
    });
}

fn llama_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.llama.temperature.clone()
    });
}

fn llama_top_p() {
    assert_number("topP", "0.9", "1", |values| values.llama.top_p.clone());
}

fn mistral_max_tokens() {
    assert_i64("max_tokens", 5, 9, |values| values.mistral.max_tokens);
}

fn mistral_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.mistral.temperature.clone()
    });
}

fn mistral_top_p() {
    assert_number("top_p", "0.9", "1", |values| values.mistral.top_p.clone());
}

fn mistral_top_k() {
    assert_number("top_k", "40", "20", |values| values.mistral.top_k.clone());
}

fn mistral_stop() {
    assert_list("stop", &["END"], &["STOP"], |values| {
        values.mistral.stop.clone()
    });
}

fn titan_max_token_count() {
    assert_i64("maxTokenCount", 32, 64, |values| {
        values.titan.max_token_count
    });
}

fn titan_stop_sequences() {
    assert_list("stopSequences", &["alpha", "beta"], &["gamma"], |values| {
        values.titan.stop_sequences.clone()
    });
}

fn titan_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.titan.temperature.clone()
    });
}

fn titan_top_p() {
    assert_number("topP", "0.9", "1", |values| values.titan.top_p.clone());
}

fn anthropic_max_tokens_to_sample() {
    let default = resolved_from("");
    assert_eq!(
        default.anthropic.max_tokens_to_sample,
        Some(4096),
        "the class field starts from litellm.max_tokens"
    );
    let configured = load(&both("max_tokens_to_sample", "64", "128")).unwrap();
    assert_eq!(
        resolved(&configured, 0).anthropic.max_tokens_to_sample,
        Some(64),
        "a deployment value wins over the module value"
    );
    assert_eq!(
        resolved(&configured, 1).anthropic.max_tokens_to_sample,
        Some(128),
        "a deployment without one takes the module value"
    );
    let environment = load_with("", &[("DEFAULT_MAX_TOKENS", "1000")]).unwrap();
    assert_eq!(
        resolved(&environment, 0).anthropic.max_tokens_to_sample,
        Some(1000),
        "DEFAULT_MAX_TOKENS sets the starting value"
    );
    let explicit = load_with(
        &global("max_tokens_to_sample: 12"),
        &[("DEFAULT_MAX_TOKENS", "1000")],
    )
    .unwrap();
    assert_eq!(
        resolved(&explicit, 0).anthropic.max_tokens_to_sample,
        Some(12),
        "an explicit value wins over the environment default"
    );
    let zero = resolved_from(&deployment("max_tokens_to_sample: 0"));
    assert_eq!(
        zero.anthropic.max_tokens_to_sample,
        Some(0),
        "an explicit zero is used as given"
    );
}

fn anthropic_stop_sequences() {
    assert_list("stop_sequences", &["END"], &["STOP"], |values| {
        values.anthropic.stop_sequences.clone()
    });
}

fn anthropic_temperature() {
    assert_number("temperature", "0.5", "0.25", |values| {
        values.anthropic.temperature.clone()
    });
}

fn anthropic_top_k() {
    assert_number("top_k", "4", "8", |values| values.anthropic.top_k.clone());
}

fn anthropic_top_p() {
    assert_number("top_p", "0.9", "1", |values| values.anthropic.top_p.clone());
}

fn anthropic_version() {
    assert_text(
        "anthropic_version",
        "bedrock-2023-05-31",
        "bedrock-2024-01-01",
        |values| values.anthropic.anthropic_version.clone(),
    );
    let referenced = resolved_with_reference(
        "anthropic_version",
        "ANTHROPIC_VERSION",
        "bedrock-2023-05-31",
    );
    assert_eq!(
        referenced.anthropic.anthropic_version.as_deref(),
        Some("bedrock-2023-05-31"),
        "an os.environ reference reads into the field"
    );
}

fn rejects_unrelated_key() {
    let error = load(&deployment("maxTokens_unrelated: 1")).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.maxTokens_unrelated"]),
        "{error}"
    );
    let global_error =
        load("litellm_settings: {bedrock_legacy_models_unrelated: true}\n").unwrap_err();
    assert!(
        matches!(&global_error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.bedrock_legacy_models_unrelated"]),
        "{global_error}"
    );
}

#[rstest]
#[case::loads_amazonai21config_maxtokens(ai21_max_tokens)]
#[case::loads_amazonai21config_temperature(ai21_temperature)]
#[case::loads_amazonai21config_topp(ai21_top_p)]
#[case::loads_amazonai21config_stopsequences(ai21_stop_sequences)]
#[case::loads_amazonai21config_frequencepenalty(ai21_frequence_penalty)]
#[case::loads_amazonai21config_presencepenalty(ai21_presence_penalty)]
#[case::loads_amazonai21config_countpenalty(ai21_count_penalty)]
#[case::loads_amazoncohereconfig_max_tokens(cohere_max_tokens)]
#[case::loads_amazoncohereconfig_temperature(cohere_temperature)]
#[case::loads_amazoncohereconfig_return_likelihood(cohere_return_likelihood)]
#[case::loads_amazonllamaconfig_maxtokencount(llama_max_token_count)]
#[case::loads_amazonllamaconfig_temperature(llama_temperature)]
#[case::loads_amazonllamaconfig_topp(llama_top_p)]
#[case::loads_amazonmistralconfig_max_tokens(mistral_max_tokens)]
#[case::loads_amazonmistralconfig_temperature(mistral_temperature)]
#[case::loads_amazonmistralconfig_top_p(mistral_top_p)]
#[case::loads_amazonmistralconfig_top_k(mistral_top_k)]
#[case::loads_amazonmistralconfig_stop(mistral_stop)]
#[case::loads_amazontitanconfig_maxtokencount(titan_max_token_count)]
#[case::loads_amazontitanconfig_stopsequences(titan_stop_sequences)]
#[case::loads_amazontitanconfig_temperature(titan_temperature)]
#[case::loads_amazontitanconfig_topp(titan_top_p)]
#[case::loads_amazonanthropicconfig_max_tokens_to_sample(anthropic_max_tokens_to_sample)]
#[case::loads_amazonanthropicconfig_stop_sequences(anthropic_stop_sequences)]
#[case::loads_amazonanthropicconfig_temperature(anthropic_temperature)]
#[case::loads_amazonanthropicconfig_top_k(anthropic_top_k)]
#[case::loads_amazonanthropicconfig_top_p(anthropic_top_p)]
#[case::loads_amazonanthropicconfig_anthropic_version(anthropic_version)]
#[case::rejects_unrelated_key(rejects_unrelated_key)]
fn bedrock_legacy_models_config_cases(#[case] run: fn()) {
    run();
}
