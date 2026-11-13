use rstest::rstest;

use super::support::settings;
use crate::api::messages_native_prepare::url::messages_url;

#[rstest]
#[case::default_base(
    None,
    "https://api.anthropic.com",
    "https://api.anthropic.com/v1/messages"
)]
#[case::configured_base(
    Some("https://configured.test"),
    "https://api.anthropic.com",
    "https://configured.test/v1/messages"
)]
#[case::suffixed_base(
    Some("https://configured.test/v1/messages"),
    "https://api.anthropic.com",
    "https://configured.test/v1/messages"
)]
#[case::trailing_slash(
    Some("https://configured.test/"),
    "https://api.anthropic.com",
    "https://configured.test//v1/messages"
)]
#[case::empty_explicit_base(Some(""), "https://env.test", "https://env.test/v1/messages")]
#[case::environment_base(None, "https://env.test", "https://env.test/v1/messages")]
#[case::environment_base_with_suffix(
    None,
    "https://env.test/v1/messages",
    "https://env.test/v1/messages"
)]
fn native_url_suffix(
    #[case] api_base: Option<&str>,
    #[case] environment_base: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        messages_url(api_base, &settings(environment_base)),
        expected
    );
}
