#![cfg(test)]

use betterlite_models::key_digest::hashing::hash_token;
use rstest::rstest;

use super::{is_non_secret_key_value, redact_logged_api_key};

const SK_ABC: &str = "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0";
const JWT: &str = "hashed-jwt-a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1";

enum Expected<'expected> {
    Absent,
    Kept(&'expected str),
    Hashed(&'expected str),
}

impl Expected<'_> {
    fn into_option(self) -> Option<String> {
        match self {
            Self::Absent => None,
            Self::Kept(value) => Some(value.to_owned()),
            Self::Hashed(value) => Some(hash_token(value)),
        }
    }
}

#[rstest]
#[case::absent(None, false, None, Expected::Absent)]
#[case::empty(Some(""), false, None, Expected::Absent)]
#[case::bearer_only(Some("Bearer "), false, None, Expected::Absent)]
#[case::bearer_prefix(Some("Bearer sk-abc"), false, None, Expected::Hashed("sk-abc"))]
#[case::lowercase_bearer(Some("bearer sk-abc"), false, None, Expected::Hashed("sk-abc"))]
#[case::uppercase_bearer(Some("BEARER sk-abc"), false, None, Expected::Hashed("sk-abc"))]
#[case::one_bearer_only(
    Some("Bearer Bearer sk-abc"),
    false,
    None,
    Expected::Hashed("Bearer sk-abc")
)]
#[case::secret_stays_hashed(Some("sk-abc"), true, None, Expected::Hashed("sk-abc"))]
#[case::digest_is_rehashed_untrusted(Some(SK_ABC), false, None, Expected::Hashed(SK_ABC))]
#[case::digest_survives_when_trusted(Some(SK_ABC), true, None, Expected::Kept(SK_ABC))]
#[case::alias_survives_when_trusted(
    Some("litellm_proxy_master_key"),
    true,
    None,
    Expected::Kept("litellm_proxy_master_key")
)]
#[case::alias_is_hashed_untrusted(
    Some("litellm_proxy_master_key"),
    false,
    None,
    Expected::Hashed("litellm_proxy_master_key")
)]
#[case::jwt_survives_when_trusted(Some(JWT), true, None, Expected::Kept(JWT))]
#[case::short_jwt_is_hashed(Some("hashed-jwt-abc"), true, None, Expected::Hashed("hashed-jwt-abc"))]
#[case::cli_session_needs_the_alias(
    Some("cli-session-abc"),
    true,
    Some("cli-session-abc"),
    Expected::Kept("cli-session-abc")
)]
#[case::cli_session_other_alias_is_hashed(
    Some("cli-session-abc"),
    true,
    Some("cli-session-xyz"),
    Expected::Hashed("cli-session-abc")
)]
#[case::cli_session_without_alias_is_hashed(
    Some("cli-session-abc"),
    true,
    None,
    Expected::Hashed("cli-session-abc")
)]
fn redacts_the_logged_api_key(
    #[case] value: Option<&str>,
    #[case] already_redacted: bool,
    #[case] key_alias: Option<&str>,
    #[case] expected: Expected<'_>,
) {
    assert_eq!(
        redact_logged_api_key(value, already_redacted, key_alias),
        expected.into_option()
    );
}

#[rstest]
#[case::alias("litellm_proxy_master_key", None, true)]
#[case::health_alias("litellm-internal-health-check", None, true)]
#[case::cli_alias("litellm-cli", None, true)]
#[case::digest(SK_ABC, None, true)]
#[case::uppercase_digest(
    "A0B1C2D3E4F5A6B7C8D9E0F1A2B3C4D5E6F7A8B9C0D1E2F3A4B5C6D7E8F9A0B1",
    None,
    true
)]
#[case::jwt(JWT, None, true)]
#[case::short_jwt("hashed-jwt-abc", None, false)]
#[case::non_hex_jwt("hashed-jwt-zzzz", None, false)]
#[case::cli_session("cli-session-abc", Some("cli-session-abc"), true)]
#[case::cli_session_other_alias("cli-session-abc", Some("cli-session-xyz"), false)]
#[case::cli_session_prefix_only("cli-session-", Some("cli-session-"), true)]
#[case::secret("sk-abc", None, false)]
#[case::empty("", None, false)]
fn classifies_non_secret_values(
    #[case] value: &str,
    #[case] alias: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_non_secret_key_value(value, alias), expected);
}
