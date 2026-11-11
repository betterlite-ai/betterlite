#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::full_model_metadata::{DEFAULT_MAX_LRU_CACHE_SIZE, Settings};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .full_model_metadata)
}

fn deployment_source(params: &str) -> String {
    format!(
        "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n{params}"
    )
}

#[rstest]
#[case::loads_default_max_lru_cache_size(true)]
fn full_model_metadata_config_cases(#[case] default_max_lru_cache_size: bool) {
    assert!(default_max_lru_cache_size);
    assert_eq!(DEFAULT_MAX_LRU_CACHE_SIZE, 64);
    let default = load("", &[]).unwrap();
    assert_eq!(default.default_max_lru_cache_size, 64);
    assert!(default.deployment_max_lru_cache_sizes.is_empty());

    let environment = load("", &[("DEFAULT_MAX_LRU_CACHE_SIZE", "8")]).unwrap();
    assert_eq!(environment.default_max_lru_cache_size, 8);
    let empty = load("", &[("DEFAULT_MAX_LRU_CACHE_SIZE", "")]).unwrap();
    assert_eq!(empty.default_max_lru_cache_size, 64);
    assert!(matches!(
        load("", &[("DEFAULT_MAX_LRU_CACHE_SIZE", "many")]),
        Err(ConfigError::Environment { .. })
    ));

    // a global litellm_settings value overrides the environment variable
    let global = load(
        "litellm_settings:\n  DEFAULT_MAX_LRU_CACHE_SIZE: 12\n",
        &[("DEFAULT_MAX_LRU_CACHE_SIZE", "8")],
    )
    .unwrap();
    assert_eq!(global.default_max_lru_cache_size, 12);
    let referenced = load(
        "litellm_settings:\n  DEFAULT_MAX_LRU_CACHE_SIZE: os.environ/LRU_SIZE\n",
        &[("LRU_SIZE", "5")],
    )
    .unwrap();
    assert_eq!(referenced.default_max_lru_cache_size, 5);
    assert!(matches!(
        load(
            "litellm_settings:\n  DEFAULT_MAX_LRU_CACHE_SIZE: many\n",
            &[]
        ),
        Err(ConfigError::InvalidInteger { .. })
    ));

    // deployment litellm_params override the global value and keep index alignment
    let deployment = load(
        "model_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      DEFAULT_MAX_LRU_CACHE_SIZE: 3\n  - model_name: b\n    litellm_params:\n      model: openai/gpt-4o\n",
        &[],
    )
    .unwrap();
    assert_eq!(deployment.default_max_lru_cache_size, 64);
    assert_eq!(
        deployment.deployment_max_lru_cache_sizes.to_vec(),
        vec![3, 64]
    );
    let override_global = load(
        "litellm_settings:\n  DEFAULT_MAX_LRU_CACHE_SIZE: 9\nmodel_list:\n  - model_name: a\n    litellm_params:\n      model: openai/gpt-4o\n      DEFAULT_MAX_LRU_CACHE_SIZE: 3\n",
        &[],
    )
    .unwrap();
    assert_eq!(override_global.default_max_lru_cache_size, 9);
    assert_eq!(
        override_global.deployment_max_lru_cache_sizes.to_vec(),
        vec![3]
    );
    assert!(matches!(
        load(
            &deployment_source("      DEFAULT_MAX_LRU_CACHE_SIZE: true\n"),
            &[]
        ),
        Err(ConfigError::InvalidScalar { .. })
    ));
}

#[rstest]
#[case::unrelated("litellm_settings:\n  unrelated_default_cache_size: 1\n")]
#[case::lowercase("litellm_settings:\n  default_max_lru_cache_size: 1\n")]
#[case::deployment_lowercase(
    "model_list:\n- model_name: a\n  litellm_params:\n    default_max_lru_cache_size: 1\n"
)]
fn rejects_unrelated_full_model_metadata_key(#[case] source: &str) {
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
