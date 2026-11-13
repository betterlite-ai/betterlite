#![cfg(test)]

use super::support::settings;
use crate::aws_settings::sources::auth_selector_presence;

#[rstest::rstest]
#[case::web_token_only("aws_web_identity_token: ''", [false; 5])]
#[case::role_only("aws_role_name: ''", [false, true, false, false, false])]
#[case::session_name_only("aws_session_name: ''", [false; 5])]
#[case::access_key_only("aws_access_key_id: ''", [false; 5])]
#[case::secret_key_only("aws_secret_access_key: ''", [false; 5])]
#[case::session_token_only("aws_session_token: ''", [false; 5])]
#[case::region_only("aws_region_name: ''", [false; 5])]
#[case::profile_only("aws_profile_name: ''", [false, false, true, false, false])]
#[case::web_missing_token(
    "aws_role_name: '', aws_session_name: ''",
    [false, true, false, false, false]
)]
#[case::web_missing_role(
    "aws_web_identity_token: '', aws_session_name: ''",
    [false; 5]
)]
#[case::web_missing_session(
    "aws_web_identity_token: '', aws_role_name: ''",
    [false, true, false, false, false]
)]
#[case::session_missing_access(
    "aws_secret_access_key: '', aws_session_token: ''",
    [false; 5]
)]
#[case::session_missing_secret(
    "aws_access_key_id: '', aws_session_token: ''",
    [false; 5]
)]
#[case::keys_without_token_or_region(
    "aws_access_key_id: '', aws_secret_access_key: ''",
    [false; 5]
)]
#[case::static_missing_access(
    "aws_secret_access_key: '', aws_region_name: ''",
    [false; 5]
)]
#[case::static_missing_secret(
    "aws_access_key_id: '', aws_region_name: ''",
    [false; 5]
)]
fn incomplete_credentials(#[case] parameters: &str, #[case] expected: [bool; 5]) {
    assert_eq!(auth_selector_presence(&settings(parameters)), expected);
}
