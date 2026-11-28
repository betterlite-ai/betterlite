#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::get_basic("byok-consent/get-basic")]
#[case::get_with_server_id("byok-consent/get-with-server-id")]
#[case::get_no_code_challenge("byok-consent/get-no-code-challenge")]
#[case::get_invalid_response_type("byok-consent/get-invalid-response-type")]
#[case::get_invalid_redirect_uri("byok-consent/get-invalid-redirect-uri")]
#[case::post_basic("byok-consent/post-basic")]
#[case::post_no_session("byok-consent/post-no-session")]
#[case::post_revoked_session("byok-consent/post-revoked-session")]
#[case::post_byok_token_rejected("byok-consent/post-byok-token-rejected")]
#[case::post_auth_code_limit("byok-consent/post-auth-code-limit")]
#[case::post_at_capacity("byok-consent/post-at-capacity")]
fn replays_b4677_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/mcp/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
