#![cfg(test)]

use rstest::rstest;

#[rstest]
#[case::result_union("credential-contracts/result-union")]
#[case::auth_spec_kind("credential-contracts/auth-spec-kind")]
#[case::auth_config_variants("credential-contracts/auth-config-variants")]
#[case::token_auth_normalize("credential-contracts/token-auth-normalize")]
#[case::token_auth_build_basic("credential-contracts/token-auth-build-basic")]
#[case::token_auth_build_post("credential-contracts/token-auth-build-post")]
#[case::byok_cache_key("credential-contracts/byok-cache-key")]
#[case::byok_cache_evict_stored("credential-contracts/byok-cache-evict-stored")]
#[case::byok_cache_evict_absent("credential-contracts/byok-cache-evict-absent")]
#[case::auth_config_token_exchange_defaults(
    "credential-contracts/auth-config-token-exchange-defaults"
)]
#[case::auth_config_aws_sigv4_defaults("credential-contracts/auth-config-aws-sigv4-defaults")]
fn replays_b4650_against_litellm(#[case] name: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<crate::case::HelperCase>(&source).unwrap();
    let result = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(result, crate::helpers::HelperRun::Match));
}
