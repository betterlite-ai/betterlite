#![cfg(test)]

use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use std::collections::BTreeMap;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .local_ttl_cache)
}

#[rstest]
#[case::loads_max_size_per_item_in_memory_cache_in_kb(false)]
#[case::loads_default_in_memory_ttl(true)]
fn local_ttl_cache_config_cases(#[case] ttl: bool) {
    let default = load("", &[]).unwrap();
    assert_eq!(super::Settings::default(), default);
    assert_eq!(default.global_max_item_kib, 1024);
    assert!(default.global_ttl.is_none());
    if ttl {
        assert_eq!(
            load("litellm_settings:\n  default_in_memory_ttl: 2.5\n", &[])
                .unwrap()
                .global_ttl,
            Some(2.5_f64)
        );
        assert!(matches!(
            load("litellm_settings:\n  default_in_memory_ttl: true\n", &[]),
            Err(ConfigError::InvalidScalar { .. })
        ));
        assert_eq!(
            load("litellm_settings:\n  default_in_memory_ttl: 7\n", &[])
                .unwrap()
                .global_ttl,
            Some(7.0_f64)
        );
        assert_eq!(
            load(
                "litellm_settings:\n  default_in_memory_ttl: os.environ/CACHE_TTL\n",
                &[("CACHE_TTL", "11")]
            )
            .unwrap()
            .global_ttl,
            Some(11.0_f64)
        );
        assert!(matches!(
            load("litellm_settings:\n  default_in_memory_ttl: invalid\n", &[]),
            Err(ConfigError::InvalidFloat { .. })
        ));
    } else {
        assert_eq!(
            load("", &[("MAX_SIZE_PER_ITEM_IN_MEMORY_CACHE_IN_KB", "7")])
                .unwrap()
                .global_max_item_kib,
            7
        );
        assert!(matches!(
            load("", &[("MAX_SIZE_PER_ITEM_IN_MEMORY_CACHE_IN_KB", "bad")]),
            Err(ConfigError::InvalidInteger { .. })
        ));
    }
}

#[rstest]
#[case::unrelated("litellm_settings:\n  unrelated_local_cache_key: 1\n")]
#[case::yaml_size("litellm_settings:\n  max_size_per_item_in_memory_cache_in_kb: 1\n")]
#[case::deployment_ttl(
    "model_list:\n- model_name: a\n  litellm_params:\n    default_in_memory_ttl: 9\n"
)]
fn rejects_unrelated_local_ttl_cache_key(#[case] source: &str) {
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
