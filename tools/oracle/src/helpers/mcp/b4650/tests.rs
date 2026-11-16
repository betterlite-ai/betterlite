#![cfg(test)]

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use std::fs;
use std::path::Path;

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
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
