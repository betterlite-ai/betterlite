use rstest::rstest;

use crate::compresr_recovery::scope::caller_scope;

use super::support::map;

#[rstest]
#[case::params_metadata(
    r#"{"litellm_params":{"metadata":{"user_api_key_auth":{"api_key":"params"}}}}"#,
    "params"
)]
#[case::params_litellm_metadata(
    r#"{"litellm_params":{"litellm_metadata":{"user_api_key_auth":{"api_key":"params"}}}}"#,
    "params"
)]
#[case::top_metadata(r#"{"metadata":{"user_api_key_auth":{"api_key":"top"}}}"#, "top")]
#[case::top_litellm_metadata(
    r#"{"litellm_metadata":{"user_api_key_auth":{"api_key":"top"}}}"#,
    "top"
)]
#[case::params_come_first(
    r#"{"litellm_params":{"metadata":{"user_api_key_auth":{"api_key":"params"}}},"metadata":{"user_api_key_auth":{"api_key":"top"}}}"#,
    "params"
)]
#[case::metadata_before_litellm_metadata(
    r#"{"metadata":{"user_api_key_auth":{"api_key":"plain"}},"litellm_metadata":{"user_api_key_auth":{"api_key":"other"}}}"#,
    "plain"
)]
#[case::bare_user_api_key_ignored(r#"{"metadata":{"user_api_key":"client"}}"#, "")]
#[case::empty_api_key_ignored(r#"{"metadata":{"user_api_key_auth":{"api_key":""}}}"#, "")]
#[case::empty_key_falls_through(
    r#"{"litellm_params":{"metadata":{"user_api_key_auth":{"api_key":""}}},"metadata":{"user_api_key_auth":{"api_key":"top"}}}"#,
    "top"
)]
#[case::non_text_api_key_ignored(r#"{"metadata":{"user_api_key_auth":{"api_key":5}}}"#, "")]
#[case::auth_not_an_object(r#"{"metadata":{"user_api_key_auth":"client"}}"#, "")]
#[case::metadata_not_an_object(
    r#"{"metadata":"client","litellm_metadata":{"user_api_key_auth":{"api_key":"top"}}}"#,
    "top"
)]
#[case::params_not_an_object(
    r#"{"litellm_params":"client","metadata":{"user_api_key_auth":{"api_key":"top"}}}"#,
    "top"
)]
#[case::no_carrier(r#"{"max_tokens":5}"#, "")]
#[case::empty_kwargs("{}", "")]
fn caller_scope_reads_only_the_authenticated_key_scoped(
    #[case] kwargs: &str,
    #[case] expected: &str,
) {
    assert_eq!(caller_scope(&map(kwargs)), expected);
}
