use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::secret;
use crate::local_servers::ProviderEnvironment;
use crate::watsonx_chat::provider::provider_info;

fn environment(api_base: Option<&str>, api_key: Option<&str>) -> ProviderEnvironment {
    ProviderEnvironment {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(secret),
    }
}

#[rstest]
#[case::explicit_wins(
    Some("https://explicit"),
    Some("https://env"),
    Some("https://explicit")
)]
#[case::environment_fills_in(None, Some("https://env"), Some("https://env"))]
#[case::empty_argument_falls_through(Some(""), Some("https://env"), Some("https://env"))]
#[case::empty_environment_is_unset(None, Some(""), None)]
#[case::nothing_set(None, None, None)]
fn provider_info_resolves_the_base(
    #[case] api_base: Option<&str>,
    #[case] env_base: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let found = provider_info(api_base, None, &environment(env_base, None));
    assert_eq!(found.api_base.as_deref(), expected);
}

#[rstest]
#[case::explicit_wins(Some("explicit"), Some("env"), "explicit")]
#[case::environment_fills_in(None, Some("env"), "env")]
#[case::empty_argument_falls_through(Some(""), Some("env"), "env")]
#[case::empty_environment_is_unset(None, Some(""), "")]
#[case::vllm_needs_no_key(None, None, "")]
fn provider_info_resolves_the_key(
    #[case] api_key: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] expected: &str,
) {
    let given = api_key.map(secret);
    let found = provider_info(None, given.as_ref(), &environment(None, env_key));
    assert_eq!(found.api_key.expose_secret(), expected);
}
