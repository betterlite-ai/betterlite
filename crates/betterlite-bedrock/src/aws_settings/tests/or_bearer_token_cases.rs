use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_settings::or_bearer_token;

#[rstest]
#[case::explicit_wins(Some("explicit-token"), Some("env-token"), Some("explicit-token"))]
#[case::environment_fills_a_missing_key(None, Some("env-token"), Some("env-token"))]
#[case::an_empty_key_falls_through(Some(""), Some("env-token"), Some("env-token"))]
#[case::explicit_wins_over_an_empty_environment(
    Some("explicit-token"),
    Some(""),
    Some("explicit-token")
)]
#[case::an_all_empty_pair_answers_the_last_operand(Some(""), Some(""), Some(""))]
#[case::both_absent_answer_none(None, None, None)]
fn or_bearer_token_prefers_the_first_non_empty_operand(
    #[case] api_key: Option<&str>,
    #[case] environment_token: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let environment = environment_token.map(|text| SecretString::from(text.to_owned()));
    let selected = or_bearer_token(&[key.as_ref(), environment.as_ref()])
        .map(|token| token.expose_secret().to_owned());
    assert_eq!(selected.as_deref(), expected);
}
