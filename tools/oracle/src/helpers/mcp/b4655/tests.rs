#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::cookie_missing("session-admission/cookie-missing")]
#[case::cookie_invalid_token("session-admission/cookie-invalid-token")]
#[case::cookie_wrong_signature("session-admission/cookie-wrong-signature")]
#[case::cookie_missing_exp("session-admission/cookie-missing-exp")]
#[case::cookie_expired("session-admission/cookie-expired")]
#[case::cookie_wrong_type("session-admission/cookie-wrong-type")]
#[case::cookie_missing_login_method("session-admission/cookie-missing-login-method")]
#[case::cookie_missing_user_id("session-admission/cookie-missing-user-id")]
#[case::cookie_empty_user_id("session-admission/cookie-empty-user-id")]
#[case::cookie_with_key_claim("session-admission/cookie-with-key-claim")]
#[case::cookie_blob_token_no_key("session-admission/cookie-blob-token-no-key")]
#[case::cookie_sk_key_revoked("session-admission/cookie-sk-key-revoked")]
#[case::cookie_valid_basic("session-admission/cookie-valid-basic")]
fn replays_b4655_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
