#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }.load(source)?.sections.auth_cache)
}

fn ttl(source: &str) -> Result<f64, ConfigError> {
    load(source, &[]).map(|settings| settings.gateway_memory_ttl)
}

fn error_category(source: &str) -> &'static str {
    match load(source, &[]) {
        Ok(_settings) => "loaded",
        Err(error) => error.category(),
    }
}

#[test]
fn fallback_env_default_60() {
    assert_eq!(
        (load("", &[]).unwrap().management_object_ttl).to_bits(),
        60.0_f64.to_bits()
    );
}

#[test]
fn fallback_env_override() {
    assert_eq!(
        (load(
            "",
            &[("DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL", "45")]
        )
        .unwrap()
        .management_object_ttl)
            .to_bits(),
        45.0_f64.to_bits()
    );
}

#[rstest]
#[case::nonnumeric("soon")]
#[case::empty("")]
#[case::fractional("60.5")]
#[case::blank("  ")]
fn fallback_env_bad_or_empty_fails_startup(#[case] value: &str) {
    let result = load(
        "",
        &[("DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL", value)],
    );
    match result {
        Err(ConfigError::InvalidInteger { path, .. }) => {
            assert_eq!(path, "DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL");
        }
        other => panic!("{value:?} should fail startup: {other:?}"),
    }
}

#[test]
fn fallback_env_zero_negative_kept() {
    assert_eq!(
        (load(
            "",
            &[("DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL", "0")]
        )
        .unwrap()
        .management_object_ttl)
            .to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        (load(
            "",
            &[("DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL", "-5")]
        )
        .unwrap()
        .management_object_ttl)
            .to_bits(),
        (-5.0_f64).to_bits()
    );
}

#[test]
fn gateway_memory_ttl_default_60() {
    let settings = load("", &[]).unwrap();
    assert_eq!((settings.gateway_memory_ttl).to_bits(), 60.0_f64.to_bits());
    assert_eq!(settings.gateway_redis_ttl, None);
    let explicit_null = load("general_settings:\n  user_api_key_cache_ttl: null\n", &[]).unwrap();
    assert_eq!(
        (explicit_null.gateway_memory_ttl).to_bits(),
        60.0_f64.to_bits()
    );
    assert_eq!(explicit_null.gateway_redis_ttl, None);
}

#[test]
fn user_ttl_updates_both_tiers() {
    let settings = load("general_settings:\n  user_api_key_cache_ttl: 30\n", &[]).unwrap();
    assert_eq!((settings.gateway_memory_ttl).to_bits(), 30.0_f64.to_bits());
    assert_eq!(settings.gateway_redis_ttl, Some(30.0_f64));
    let fractional = load("general_settings:\n  user_api_key_cache_ttl: 30.5\n", &[]).unwrap();
    assert_eq!(
        (fractional.gateway_memory_ttl).to_bits(),
        30.5_f64.to_bits()
    );
    assert_eq!(fractional.gateway_redis_ttl, Some(30.5_f64));
    let text = load("general_settings:\n  user_api_key_cache_ttl: \"30\"\n", &[]).unwrap();
    assert_eq!((text.gateway_memory_ttl).to_bits(), 30.0_f64.to_bits());
    assert_eq!(text.gateway_redis_ttl, Some(30.0_f64));
}

#[test]
fn user_ttl_none_leaves_defaults() {
    let settings = load("general_settings:\n  user_api_key_cache_ttl: null\n", &[]).unwrap();
    assert_eq!((settings.gateway_memory_ttl).to_bits(), 60.0_f64.to_bits());
    assert_eq!(settings.gateway_redis_ttl, None);
}

#[test]
fn user_ttl_zero_negative_fractional() {
    assert_eq!(
        (ttl("general_settings:\n  user_api_key_cache_ttl: 0\n").unwrap()).to_bits(),
        0.0_f64.to_bits()
    );
    assert_eq!(
        (ttl("general_settings:\n  user_api_key_cache_ttl: -2.5\n").unwrap()).to_bits(),
        (-2.5_f64).to_bits()
    );
    assert_eq!(
        (ttl("general_settings:\n  user_api_key_cache_ttl: 0.25\n").unwrap()).to_bits(),
        0.25_f64.to_bits()
    );
}

#[rstest]
#[case::text("general_settings:\n  user_api_key_cache_ttl: soon\n", "invalid_float")]
#[case::boolean(
    "general_settings:\n  user_api_key_cache_ttl: true\n",
    "invalid_scalar"
)]
#[case::sequence("general_settings:\n  user_api_key_cache_ttl: [1]\n", "invalid_scalar")]
fn malformed_user_ttl_fails(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(error_category(source), expected);
}

#[test]
fn max_size_default_none_maps_200() {
    // B4293's update_in_memory_max_size(None) restores the 200-entry default, so None passes through
    assert_eq!(load("", &[]).unwrap().user_api_key_cache_max_size, None);
    assert_eq!(
        load(
            "general_settings:\n  user_api_key_cache_max_size: null\n",
            &[]
        )
        .unwrap()
        .user_api_key_cache_max_size,
        None
    );
}

#[test]
fn max_size_positive_integral_coercion() {
    assert_eq!(
        load(
            "general_settings:\n  user_api_key_cache_max_size: 250\n",
            &[]
        )
        .unwrap()
        .user_api_key_cache_max_size,
        Some(250)
    );
    assert_eq!(
        load(
            "general_settings:\n  user_api_key_cache_max_size: 250.0\n",
            &[]
        )
        .unwrap()
        .user_api_key_cache_max_size,
        Some(250)
    );
    assert_eq!(
        load(
            "general_settings:\n  user_api_key_cache_max_size: \"250\"\n",
            &[]
        )
        .unwrap()
        .user_api_key_cache_max_size,
        Some(250)
    );
}

#[rstest]
#[case::zero("general_settings:\n  user_api_key_cache_max_size: 0\n", "yaml")]
#[case::negative("general_settings:\n  user_api_key_cache_max_size: -3\n", "yaml")]
#[case::fractional("general_settings:\n  user_api_key_cache_max_size: 2.5\n", "yaml")]
#[case::text_zero("general_settings:\n  user_api_key_cache_max_size: \"0\"\n", "yaml")]
#[case::boolean(
    "general_settings:\n  user_api_key_cache_max_size: true\n",
    "invalid_scalar"
)]
fn max_size_zero_negative_fractional_rejected(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(error_category(source), expected);
}

#[rstest]
#[case::infinity("general_settings:\n  user_api_key_cache_ttl: .inf\n")]
#[case::negative_infinity("general_settings:\n  user_api_key_cache_ttl: -.inf\n")]
#[case::nan("general_settings:\n  user_api_key_cache_ttl: .nan\n")]
#[case::huge("general_settings:\n  user_api_key_cache_ttl: 1e300\n")]
#[case::negative_huge("general_settings:\n  user_api_key_cache_ttl: -1e300\n")]
fn nonfinite_or_unrepresentable_ttl_rejected(#[case] source: &str) {
    assert_eq!(error_category(source), "yaml");
}
