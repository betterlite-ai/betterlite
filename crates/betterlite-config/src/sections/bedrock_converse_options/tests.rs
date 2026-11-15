#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::Number;

use super::{ConverseOptionValues, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str) -> Result<Settings, ConfigError> {
    Ok(ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)?
    .sections
    .bedrock_converse_options)
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

fn assert_i64(
    key: &str,
    deployment_value: i64,
    global_value: i64,
    read: fn(&ConverseOptionValues) -> Option<i64>,
) {
    let default = load("").unwrap();
    assert_eq!(read(&default.litellm_settings), None, "{key} default");
    assert!(default.deployments.is_empty());
    let only_deployment = load(&deployment(&format!("{key}: {deployment_value}"))).unwrap();
    assert_eq!(
        read(only_deployment.deployments.first().unwrap()),
        Some(deployment_value),
        "{key} deployment"
    );
    let only_global = load(&global(&format!("{key}: {global_value}"))).unwrap();
    assert_eq!(
        read(&only_global.litellm_settings),
        Some(global_value),
        "{key} global"
    );
    let configured = load(&both(
        key,
        &deployment_value.to_string(),
        &global_value.to_string(),
    ))
    .unwrap();
    assert_eq!(
        read(configured.deployments.first().unwrap()),
        Some(deployment_value),
        "{key} keeps the deployment value"
    );
    assert_eq!(
        read(&configured.litellm_settings),
        Some(global_value),
        "{key} keeps the module value"
    );
    assert_eq!(
        read(&configured.resolve(0)),
        Some(deployment_value),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        read(&configured.resolve(1)),
        Some(global_value),
        "{key} without a deployment"
    );
}

fn assert_number(
    key: &str,
    deployment_value: &str,
    global_value: &str,
    read: fn(&ConverseOptionValues) -> Option<Number>,
) {
    let rendered = |values: &ConverseOptionValues| read(values).map(|number| number.to_string());
    let default = load("").unwrap();
    assert_eq!(rendered(&default.litellm_settings), None, "{key} default");
    let only_deployment = load(&deployment(&format!("{key}: {deployment_value}"))).unwrap();
    assert_eq!(
        rendered(only_deployment.deployments.first().unwrap()),
        Some(deployment_value.to_owned()),
        "{key} deployment"
    );
    let only_global = load(&global(&format!("{key}: {global_value}"))).unwrap();
    assert_eq!(
        rendered(&only_global.litellm_settings),
        Some(global_value.to_owned()),
        "{key} global"
    );
    let configured = load(&both(key, deployment_value, global_value)).unwrap();
    assert_eq!(
        rendered(configured.deployments.first().unwrap()),
        Some(deployment_value.to_owned()),
        "{key} keeps the deployment value"
    );
    assert_eq!(
        rendered(&configured.litellm_settings),
        Some(global_value.to_owned()),
        "{key} keeps the module value"
    );
    assert_eq!(
        rendered(&configured.resolve(0)),
        Some(deployment_value.to_owned()),
        "{key} resolves the deployment over the module"
    );
    assert_eq!(
        rendered(&configured.resolve(1)),
        Some(global_value.to_owned()),
        "{key} without a deployment"
    );
}

#[derive(Clone, Copy, Debug)]
enum Field {
    MaxTokens,
    StopSequences,
    Temperature,
    TopP,
    TopK,
    RejectsUnrelatedKey,
}

#[rstest]
#[case::loads_amazonconverseconfig_maxtokens(Field::MaxTokens)]
#[case::loads_amazonconverseconfig_stopsequences(Field::StopSequences)]
#[case::loads_amazonconverseconfig_temperature(Field::Temperature)]
#[case::loads_amazonconverseconfig_topp(Field::TopP)]
#[case::loads_amazonconverseconfig_topk(Field::TopK)]
#[case::rejects_unrelated_key(Field::RejectsUnrelatedKey)]
fn bedrock_converse_options_config_cases(#[case] field: Field) {
    match field {
        Field::MaxTokens => {
            assert_i64("maxTokens", 16, 32, |values| values.max_tokens);
            let zero = load(&deployment("maxTokens: 0")).unwrap();
            assert_eq!(
                zero.deployments.first().unwrap().max_tokens,
                Some(0),
                "an explicit zero is used as given"
            );
        }
        Field::TopK => assert_i64("topK", 8, 4, |values| values.top_k),
        Field::Temperature => {
            assert_number("temperature", "0.5", "0.25", |values| {
                values.temperature.clone()
            });
            let zero = load(&deployment("temperature: 0")).unwrap();
            assert_eq!(
                zero.deployments
                    .first()
                    .unwrap()
                    .temperature
                    .as_ref()
                    .map(Number::to_string),
                Some("0".to_owned()),
                "an explicit zero is used as given"
            );
        }
        Field::TopP => assert_number("topP", "0.9", "1", |values| values.top_p.clone()),
        Field::StopSequences => {
            let default = load("").unwrap();
            assert_eq!(default.litellm_settings.stop_sequences, None);
            let only_deployment = load(&deployment("stopSequences: [alpha, beta]")).unwrap();
            assert_eq!(
                only_deployment.deployments.first().unwrap().stop_sequences,
                Some(vec!["alpha".to_owned(), "beta".to_owned()])
            );
            let only_global = load(&global("stopSequences: [gamma]")).unwrap();
            assert_eq!(
                only_global.litellm_settings.stop_sequences,
                Some(vec!["gamma".to_owned()])
            );
            let configured = load(&format!(
                "model_list:\n- model_name: unit\n  litellm_params: {{model: bedrock/unit, stopSequences: [deploy]}}\nlitellm_settings: {{stopSequences: [global]}}\n"
            ))
            .unwrap();
            assert_eq!(
                configured.resolve(0).stop_sequences,
                Some(vec!["deploy".to_owned()])
            );
            assert_eq!(
                configured.resolve(1).stop_sequences,
                Some(vec!["global".to_owned()])
            );
            let empty = load(&deployment("stopSequences: []")).unwrap();
            assert_eq!(
                empty.deployments.first().unwrap().stop_sequences,
                Some(Vec::new()),
                "an explicit empty list is used as given"
            );
        }
        Field::RejectsUnrelatedKey => {
            let error = load(&deployment("maxTokens_unrelated: 1")).unwrap_err();
            assert!(
                matches!(&error, ConfigError::Unsupported { paths } if *paths == ["model_list.0.litellm_params.maxTokens_unrelated"]),
                "{error}"
            );
            let global_error =
                load("litellm_settings: {converse_options_unrelated: true}\n").unwrap_err();
            assert!(
                matches!(&global_error, ConfigError::Unsupported { paths } if *paths == ["litellm_settings.converse_options_unrelated"]),
                "{global_error}"
            );
        }
    }
}
