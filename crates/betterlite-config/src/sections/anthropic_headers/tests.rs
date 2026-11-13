#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_headers)
}

#[rstest]
#[case::loads_use_bearer_for_custom_base(0)]
#[case::loads_default_model_created_at_time(1)]
#[case::rejects_unrelated_key(2)]
fn anthropic_headers_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let default = load("", &[]).unwrap();
            assert!(!default.use_bearer_for_custom_base);
            assert!(default.deployment_bearer_opt_ins.is_empty());
            let module = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\n",
                &[],
            )
            .unwrap();
            assert!(module.use_bearer_for_custom_base);
            let deployment = "model_list:\n  - model_name: claude\n    litellm_params:\n      model: anthropic/claude-sonnet-4\n      use_bearer_for_custom_base: true\n";
            assert_eq!(
                load(deployment, &[]).unwrap().deployment_bearer_opt_ins,
                vec![true]
            );
            let inherited = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\nmodel_list:\n  - model_name: first\n  - model_name: second\n",
                &[],
            )
            .unwrap();
            assert_eq!(inherited.deployment_bearer_opt_ins, vec![true, true]);
            let overridden = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\nmodel_list:\n  - model_name: first\n    litellm_params:\n      use_bearer_for_custom_base: false\n  - model_name: second\n",
                &[],
            )
            .unwrap();
            assert_eq!(overridden.deployment_bearer_opt_ins, vec![false, true]);
            let seed = load(
                include_str!("../../../fuzz/corpus/config_yaml/anthropic_headers-use-bearer-for-custom-base.yaml"),
                &[],
            )
            .unwrap();
            assert!(seed.use_bearer_for_custom_base);
            assert_eq!(seed.deployment_bearer_opt_ins, vec![true, true]);
        }
        1 => {
            let default = load("", &[]).unwrap();
            assert_eq!(default.default_model_created_at_time, 1_677_610_602);
            let from_environment = load("", &[("DEFAULT_MODEL_CREATED_AT_TIME", "42")])
                .unwrap()
                .default_model_created_at_time;
            assert_eq!(from_environment, 42);
            let padded = load("", &[("DEFAULT_MODEL_CREATED_AT_TIME", "  42 ")])
                .unwrap()
                .default_model_created_at_time;
            assert_eq!(padded, 42);
            let empty = load("", &[("DEFAULT_MODEL_CREATED_AT_TIME", "")])
                .unwrap()
                .default_model_created_at_time;
            assert_eq!(empty, 1_677_610_602);
            let seed = load(
                include_str!("../../../fuzz/corpus/config_yaml/anthropic_headers-default-model-created-at-time.yaml"),
                &[],
            )
            .unwrap();
            assert_eq!(seed.default_model_created_at_time, 1_677_610_602);
            for value in ["abc", "3.5", "1e3", "00:01"] {
                let error = load("", &[("DEFAULT_MODEL_CREATED_AT_TIME", value)]).unwrap_err();
                assert_eq!(error.category(), "environment");
                assert!(error.to_string().contains("DEFAULT_MODEL_CREATED_AT_TIME"));
            }
        }
        _ => {
            let source = "litellm_settings:\n  anthropic_headers_unrelated_key: 1\n";
            assert!(matches!(
                load(source, &[]),
                Err(ConfigError::Unsupported { .. })
            ));
        }
    }
}
