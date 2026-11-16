use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::secret;
use crate::local_servers::ProviderEnvironment;
use crate::watsonx_chat::provider::provider_info;

#[rstest]
#[case::explicit_wins(
    (Some("https://explicit"), Some("https://env")),
    (Some("explicit"), Some("env")),
    Some("https://explicit"),
    "explicit"
)]
#[case::environment_fills_in(
    (None, Some("https://env")),
    (None, Some("env")),
    Some("https://env"),
    "env"
)]
#[case::empty_argument_falls_through(
    (Some(""), Some("https://env")),
    (Some(""), Some("env")),
    Some("https://env"),
    "env"
)]
#[case::empty_environment_is_unset((None, Some("")), (None, Some("")), None, "")]
#[case::nothing_set((None, None), (None, None), None, "")]
fn provider_info_falls_back_to_the_vllm_environment(
    #[case] base: (Option<&str>, Option<&str>),
    #[case] key: (Option<&str>, Option<&str>),
    #[case] expected_base: Option<&str>,
    #[case] expected_key: &str,
) {
    let environment = ProviderEnvironment {
        api_base: base.1.map(str::to_owned),
        api_key: key.1.map(secret),
    };
    let given_key = key.0.map(secret);
    let found = provider_info(base.0, given_key.as_ref(), &environment);
    assert_eq!(found.api_base.as_deref(), expected_base);
    assert_eq!(found.api_key.expose_secret(), expected_key);
}
