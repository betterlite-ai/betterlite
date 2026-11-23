#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{QwenValues, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<Settings, ConfigError> {
    load_with(source, &[])
}

fn load_with(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .bedrock_qwen_deepseek)
}

fn deployment(fields: &str) -> String {
    format!(
        "model_list:\n- model_name: unit\n  litellm_params: {{model: amazon_qwen3/unit, amazon_qwen3: {{{fields}}}}}\n"
    )
}

fn global(fields: &str) -> String {
    format!("litellm_settings: {{amazon_qwen3: {{{fields}}}}}\n")
}

fn assert_integer_field(key: &str, read: fn(&QwenValues) -> Option<i64>) {
    let default = load("").unwrap();
    assert_eq!(read(&default.litellm_settings), None, "{key} default");
    assert!(default.deployments.is_empty());
    let only_deployment = load(&deployment(&format!("{key}: 64"))).unwrap();
    assert_eq!(
        read(only_deployment.deployments.first().unwrap()),
        Some(64),
        "{key} deployment"
    );
    let only_global = load(&global(&format!("{key}: 128"))).unwrap();
    assert_eq!(
        read(&only_global.litellm_settings),
        Some(128),
        "{key} global"
    );
    assert_both_configured(key, read);
    let zero = load(&deployment(&format!("{key}: 0"))).unwrap();
    assert_eq!(
        read(zero.deployments.first().unwrap()),
        Some(0),
        "{key} keeps an explicit zero"
    );
}

fn assert_integer_environment(key: &str, read: fn(&QwenValues) -> Option<i64>) {
    let variable = format!("QWEN_CONFIG_{}", key.to_uppercase());
    let referenced = load_with(
        &deployment(&format!("{key}: os.environ/{variable}")),
        &[(variable.as_str(), "16")],
    )
    .unwrap();
    assert_eq!(
        read(referenced.deployments.first().unwrap()),
        Some(16),
        "{key} reads the environment"
    );
    let blank = load_with(
        &deployment(&format!("{key}: os.environ/{variable}")),
        &[(variable.as_str(), "")],
    )
    .unwrap();
    assert_eq!(
        read(blank.deployments.first().unwrap()),
        None,
        "{key} treats an empty variable as unset"
    );
    assert_integer_failures(key);
}

fn assert_both_configured(key: &str, read: fn(&QwenValues) -> Option<i64>) {
    let configured = load(&format!(
        "{}{}",
        deployment(&format!("{key}: 64")),
        global(&format!("{key}: 128"))
    ))
    .unwrap();
    assert_eq!(
        read(configured.deployments.first().unwrap()),
        Some(64),
        "{key} keeps the deployment value"
    );
    assert_eq!(
        read(&configured.litellm_settings),
        Some(128),
        "{key} keeps the module value"
    );
    assert_eq!(
        read(&configured.resolve(0)),
        Some(64),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        read(&configured.resolve(1)),
        Some(128),
        "{key} without a deployment"
    );
}

fn assert_integer_failures(key: &str) {
    let expected = format!("model_list.0.litellm_params.amazon_qwen3.{key}");
    let variable = format!("ABSENT_QWEN_CONFIG_{}", key.to_uppercase());
    let missing = load(&deployment(&format!("{key}: os.environ/{variable}"))).unwrap_err();
    assert!(
        matches!(&missing, ConfigError::MissingEnvironment { variable: name, path } if *name == variable && *path == expected),
        "{missing}"
    );
    let invalid = load(&deployment(&format!("{key}: 'many'"))).unwrap_err();
    assert!(
        matches!(&invalid, ConfigError::InvalidInteger { path, .. } if *path == expected),
        "{invalid}"
    );
}

fn assert_float_field(key: &str, read: fn(&QwenValues) -> Option<f64>) {
    assert_float_sources(key, read);
    assert_float_environment(key, read);
}

fn assert_float_sources(key: &str, read: fn(&QwenValues) -> Option<f64>) {
    let default = load("").unwrap();
    assert_eq!(read(&default.litellm_settings), None, "{key} default");
    assert!(default.deployments.is_empty());
    let only_deployment = load(&deployment(&format!("{key}: 0.2"))).unwrap();
    assert_eq!(
        read(only_deployment.deployments.first().unwrap()),
        Some(0.2),
        "{key} deployment"
    );
    let only_global = load(&global(&format!("{key}: 0.7"))).unwrap();
    assert_eq!(
        read(&only_global.litellm_settings),
        Some(0.7),
        "{key} global"
    );
    let configured = load(&format!(
        "{}{}",
        deployment(&format!("{key}: 0.2")),
        global(&format!("{key}: 0.7"))
    ))
    .unwrap();
    assert_eq!(
        read(&configured.resolve(0)),
        Some(0.2),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        read(&configured.resolve(1)),
        Some(0.7),
        "{key} without a deployment"
    );
    let zero = load(&deployment(&format!("{key}: 0.0"))).unwrap();
    assert_eq!(
        read(zero.deployments.first().unwrap()),
        Some(0.0),
        "{key} keeps an explicit zero"
    );
}

fn assert_float_environment(key: &str, read: fn(&QwenValues) -> Option<f64>) {
    let variable = format!("QWEN_CONFIG_{}", key.to_uppercase());
    let referenced = load_with(
        &deployment(&format!("{key}: os.environ/{variable}")),
        &[(variable.as_str(), "0.3")],
    )
    .unwrap();
    assert_eq!(
        read(referenced.deployments.first().unwrap()),
        Some(0.3),
        "{key} reads the environment"
    );
    let expected = format!("model_list.0.litellm_params.amazon_qwen3.{key}");
    let invalid = load(&deployment(&format!("{key}: 'many'"))).unwrap_err();
    assert!(
        matches!(&invalid, ConfigError::InvalidFloat { path, .. } if *path == expected),
        "{invalid}"
    );
}

fn loads_amazonqwen3config_max_tokens() {
    assert_integer_field("max_tokens", |values| values.max_tokens);
    assert_integer_environment("max_tokens", |values| values.max_tokens);
}

fn loads_amazonqwen3config_temperature() {
    assert_float_field("temperature", |values| values.temperature);
}

fn loads_amazonqwen3config_top_p() {
    assert_float_field("top_p", |values| values.top_p);
}

fn loads_amazonqwen3config_top_k() {
    assert_integer_field("top_k", |values| values.top_k);
    assert_integer_environment("top_k", |values| values.top_k);
}

fn loads_amazonqwen3config_stop() {
    let default = load("").unwrap();
    assert_eq!(default.litellm_settings.stop, None);
    let only_deployment = load(&deployment("stop: [END]")).unwrap();
    assert_eq!(
        only_deployment.deployments.first().unwrap().stop,
        Some(vec![String::from("END")])
    );
    let configured = load(&format!(
        "{}{}",
        deployment("stop: [END]"),
        global("stop: [STOP]")
    ))
    .unwrap();
    assert_eq!(
        configured.resolve(0).stop,
        Some(vec![String::from("END")]),
        "the deployment value wins"
    );
    assert_eq!(
        configured.resolve(1).stop,
        Some(vec![String::from("STOP")]),
        "without a deployment"
    );
    let empty = load(&deployment("stop: []")).unwrap();
    assert_eq!(
        empty.deployments.first().unwrap().stop,
        Some(Vec::new()),
        "an explicit empty list is used as given"
    );
}

fn rejects_unrelated_key() {
    let nested = load(&global("extra_knob: 1")).unwrap_err();
    assert!(
        matches!(&nested, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.amazon_qwen3.extra_knob"]),
        "{nested}"
    );
    let in_deployment = load(&deployment("extra_knob: 1")).unwrap_err();
    assert!(
        matches!(&in_deployment, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.amazon_qwen3.extra_knob"]),
        "{in_deployment}"
    );
}

#[derive(Clone, Copy, Debug)]
enum Field {
    MaxTokens,
    Temperature,
    TopP,
    TopK,
    Stop,
    RejectsUnrelatedKey,
}

#[rstest]
#[case::loads_amazonqwen3config_max_tokens(Field::MaxTokens)]
#[case::loads_amazonqwen3config_temperature(Field::Temperature)]
#[case::loads_amazonqwen3config_top_p(Field::TopP)]
#[case::loads_amazonqwen3config_top_k(Field::TopK)]
#[case::loads_amazonqwen3config_stop(Field::Stop)]
#[case::rejects_unrelated_key(Field::RejectsUnrelatedKey)]
fn bedrock_qwen_config_cases(#[case] field: Field) {
    match field {
        Field::MaxTokens => loads_amazonqwen3config_max_tokens(),
        Field::Temperature => loads_amazonqwen3config_temperature(),
        Field::TopP => loads_amazonqwen3config_top_p(),
        Field::TopK => loads_amazonqwen3config_top_k(),
        Field::Stop => loads_amazonqwen3config_stop(),
        Field::RejectsUnrelatedKey => rejects_unrelated_key(),
    }
}
