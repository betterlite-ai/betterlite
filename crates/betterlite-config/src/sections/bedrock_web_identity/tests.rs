#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::bedrock_web_identity::{DEFAULT_MAX_POLICY_SIZE, Settings};

fn load(environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .bedrock_web_identity)
}

#[derive(Debug, Clone, Copy)]
enum Scenario {
    Loads,
    Unrelated,
}

#[rstest]
#[case::loads_bedrock_max_policy_size(Scenario::Loads)]
#[case::rejects_unrelated_key(Scenario::Unrelated)]
fn bedrock_web_identity_config_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Loads => loads_bedrock_max_policy_size(),
        Scenario::Unrelated => rejects_unrelated_key(),
    }
}

fn loads_bedrock_max_policy_size() {
    let default = load(&[]).unwrap();
    assert_eq!(default, Settings::default());
    assert_eq!(default.max_policy_size, DEFAULT_MAX_POLICY_SIZE);
    assert_eq!(DEFAULT_MAX_POLICY_SIZE, 75);
    assert_eq!(
        load(&[("BEDROCK_MAX_POLICY_SIZE", "120")])
            .unwrap()
            .max_policy_size,
        120
    );
    assert_eq!(
        load(&[("BEDROCK_MAX_POLICY_SIZE", " 75 ")])
            .unwrap()
            .max_policy_size,
        75
    );
    assert_eq!(
        load(&[("BEDROCK_MAX_POLICY_SIZE", "")])
            .unwrap()
            .max_policy_size,
        DEFAULT_MAX_POLICY_SIZE
    );
    assert_eq!(
        load(&[("BEDROCK_MAX_POLICY_SIZE", "-5")])
            .unwrap()
            .max_policy_size,
        -5
    );
    assert!(matches!(
        load(&[("BEDROCK_MAX_POLICY_SIZE", "abc")]),
        Err(ConfigError::InvalidInteger { path, .. }) if path == "BEDROCK_MAX_POLICY_SIZE"
    ));
}

fn rejects_unrelated_key() {
    let lookup = BTreeMap::<String, String>::new();
    for source in [
        "litellm_settings:\n  bedrock_max_policy_size: 120\n",
        "model_list:\n- model_name: a\n  litellm_params:\n    bedrock_max_policy_size: 120\n",
    ] {
        assert!(matches!(
            ConfigLoader {
                lookup: lookup.clone()
            }
            .load(source),
            Err(ConfigError::Unsupported { .. })
        ));
    }
}
