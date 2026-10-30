#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

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
#[case::loads_max_size_per_item_in_memory_cache_in_kb("max_size_per_item_in_memory_cache_in_kb")]
#[case::loads_default_in_memory_ttl("default_in_memory_ttl")]
fn local_ttl_cache_config_cases(#[case] key: &str) {
    let default = load("", &[]).unwrap();
    assert_eq!(default.global_max_item_kib, Some(1024));
    assert!(default.global_ttl.is_none());
    let global = load(&format!("litellm_settings:\n  {key}: 7\n"), &[]).unwrap();
    if key.starts_with("max_") {
        assert_eq!(global.global_max_item_kib, Some(7));
    } else {
        assert_eq!(global.global_ttl, Some(7.0));
    }
    let deployment = load(
        &format!("model_list:\n- model_name: a\n  litellm_params:\n    {key}: 9\n"),
        &[],
    )
    .unwrap();
    if key.starts_with("max_") {
        assert_eq!(deployment.deployment_max_item_kib, vec![Some(9)]);
    } else {
        assert_eq!(deployment.deployment_ttl, vec![Some(9.0)]);
    }
    let environment = load(
        &format!("litellm_settings:\n  {key}: os.environ/CACHE_VALUE\n"),
        &[("CACHE_VALUE", "11")],
    )
    .unwrap();
    if key.starts_with("max_") {
        assert_eq!(environment.global_max_item_kib, Some(11));
    } else {
        assert_eq!(environment.global_ttl, Some(11.0));
    }
}

#[test]
fn rejects_unrelated_local_ttl_cache_key() {
    let error = load("litellm_settings:\n  unrelated_local_cache_key: 1\n", &[]).unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { .. }));
}

#[test]
fn rejects_malformed_local_ttl_cache_integer() {
    let error = load(
        "litellm_settings:\n  max_size_per_item_in_memory_cache_in_kb: invalid\n",
        &[],
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::InvalidInteger { .. }));
}
