#![cfg(test)]

use std::fs::read_to_string;
use std::sync::Arc;

use rstest::rstest;
use secrecy::SecretString;
use tracing::{Level, subscriber::with_default};
use tracing_subscriber::fmt;

use super::error::{CloudflareChatError, get_error_class};
use super::{complete_url, validate_environment};
use crate::error::{ErrorRequest, ProviderError};
use crate::openai_base_config::Headers;

const ACCOUNT_URL: &str =
    "https://api.cloudflare.com/client/v4/accounts/acct-1/ai/v1/chat/completions";
const MISSING_ACCOUNT_ID: &str = "Missing CLOUDFLARE_ACCOUNT_ID - set CLOUDFLARE_ACCOUNT_ID in the environment or pass api_base explicitly";
const MISSING_KEY: &str = "Missing Cloudflare API Key - A call is being made to cloudflare but no key is set either in the environment variables or via params";
const LEGACY_WARNING: &str = "Cloudflare api_base ending in '/ai/run' is the legacy Workers AI path and no longer serves OpenAI-compatible requests; rewriting to the '/ai/v1' endpoint";

fn headers_of(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::WARN)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, read_to_string(log.path()).unwrap())
}

#[rstest]
#[case::unset_base(None, "acct-1")]
#[case::empty_base(Some(""), "acct-1")]
#[case::whitespace_around_id(None, "  acct-1  ")]
fn account_base_resolves_from_the_account_id(
    #[case] api_base: Option<&str>,
    #[case] account_id: &str,
) {
    assert_eq!(
        complete_url(api_base, Some(account_id)).unwrap(),
        ACCOUNT_URL
    );
}

#[rstest]
#[case::plain_base(
    "https://proxy.example/v1",
    "https://proxy.example/v1/chat/completions"
)]
#[case::trailing_slash(
    "https://proxy.example/v1/",
    "https://proxy.example/v1/chat/completions"
)]
#[case::already_complete(
    "https://proxy.example/chat/completions",
    "https://proxy.example/chat/completions"
)]
#[case::complete_with_suffix(
    "https://proxy.example/chat/completions/extra",
    "https://proxy.example/chat/completions/extra"
)]
#[case::complete_with_trailing_slash(
    "https://proxy.example/chat/completions/",
    "https://proxy.example/chat/completions"
)]
#[case::double_trailing_slash(
    "https://proxy.example/custom//",
    "https://proxy.example/custom/chat/completions"
)]
#[case::spaces_only(" ", " /chat/completions")]
fn explicit_base_joins_chat_completions(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(
        complete_url(Some(api_base), Some("ignored")).unwrap(),
        expected
    );
}

#[test]
fn explicit_base_needs_no_account_id() {
    assert_eq!(
        complete_url(Some("https://proxy.example"), None).unwrap(),
        "https://proxy.example/chat/completions"
    );
}

#[rstest]
#[case::bare_run(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/chat/completions"
)]
#[case::trailing_slash(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/chat/completions"
)]
#[case::double_trailing_slash(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run//",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/chat/completions"
)]
#[case::model_path(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/@cf/meta/llama",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/@cf/meta/llama/chat/completions"
)]
#[case::complete_path(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/chat/completions",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/chat/completions"
)]
#[case::nested_v1(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/ai/run",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/ai/v1/chat/completions"
)]
fn legacy_ai_run_base_rewrites_to_ai_v1(#[case] api_base: &str, #[case] expected: &str) {
    assert_eq!(complete_url(Some(api_base), None).unwrap(), expected);
}

#[test]
fn legacy_rewrite_warns_once_with_litellm_text() {
    let (url, warnings) = capture_warnings(|| {
        complete_url(
            Some("https://api.cloudflare.com/client/v4/accounts/A/ai/run"),
            None,
        )
    });
    assert_eq!(
        url.unwrap(),
        "https://api.cloudflare.com/client/v4/accounts/A/ai/v1/chat/completions"
    );
    assert!(warnings.contains(" WARN "), "{warnings}");
    assert_eq!(warnings.matches(LEGACY_WARNING).count(), 1, "{warnings}");
}

#[rstest]
#[case::plain_base(
    "https://proxy.example/v1",
    "https://proxy.example/v1/chat/completions"
)]
#[case::model_path(
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/@cf/meta/llama",
    "https://api.cloudflare.com/client/v4/accounts/A/ai/run/@cf/meta/llama/chat/completions"
)]
fn ordinary_base_does_not_warn(#[case] api_base: &str, #[case] expected: &str) {
    let (url, warnings) = capture_warnings(|| complete_url(Some(api_base), None));
    assert_eq!(url.unwrap(), expected);
    assert!(!warnings.contains("legacy Workers AI path"), "{warnings}");
}

#[rstest]
#[case::no_base_or_id(None, None)]
#[case::empty_base_no_id(Some(""), None)]
#[case::whitespace_id(None, Some("   "))]
fn missing_account_id_reports_litellm_text(
    #[case] api_base: Option<&str>,
    #[case] account_id: Option<&str>,
) {
    let error = complete_url(api_base, account_id).unwrap_err();
    assert_eq!(error, CloudflareChatError::MissingAccountId);
    assert_eq!(error.to_string(), MISSING_ACCOUNT_ID);
}

#[rstest]
#[case::key_sets_both(Some("key"), &[], &[("Authorization", "Bearer key"), ("Content-Type", "application/json")])]
#[case::empty_key_still_counts(
    Some(""),
    &[],
    &[("Authorization", "Bearer "), ("Content-Type", "application/json")]
)]
#[case::other_headers_stay(
    Some("key"),
    &[("X-Caller", "kept")],
    &[("Authorization", "Bearer key"), ("Content-Type", "application/json"), ("X-Caller", "kept")]
)]
#[case::lowercase_content_type_blocks_default(
    Some("key"),
    &[("content-type", "text/plain")],
    &[("Authorization", "Bearer key"), ("content-type", "text/plain")]
)]
#[case::exact_content_type_blocks_default(
    Some("key"),
    &[("Content-Type", "text/plain")],
    &[("Authorization", "Bearer key"), ("Content-Type", "text/plain")]
)]
#[case::shouty_content_type_gets_second_entry(
    Some("key"),
    &[("CONTENT-TYPE", "application/xml")],
    &[("Authorization", "Bearer key"), ("CONTENT-TYPE", "application/xml"), ("Content-Type", "application/json")]
)]
#[case::authorization_is_replaced(
    Some("key"),
    &[("Authorization", "Bearer old")],
    &[("Authorization", "Bearer key"), ("Content-Type", "application/json")]
)]
fn validate_environment_matches_the_header_rule(
    #[case] api_key: Option<&str>,
    #[case] headers: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    let key = api_key.map(SecretString::from);
    assert_eq!(
        validate_environment(headers_of(headers), key.as_ref()).unwrap(),
        headers_of(expected)
    );
}

#[test]
fn missing_key_reports_litellm_text() {
    let error = validate_environment(Headers::new(), None).unwrap_err();
    assert_eq!(error, CloudflareChatError::MissingApiKey);
    assert_eq!(error.to_string(), MISSING_KEY);
}

#[test]
fn cloudflare_error_projects_to_a_provider_error_without_headers() {
    let supplied = headers_of(&[("X-Ignored", "value")]);
    let error = get_error_class("boom", 429, &supplied);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "boom");
    let projected = ProviderError::from(error);
    assert_eq!(projected.status_code, 429);
    assert_eq!(projected.message, "boom");
    assert_eq!(projected.headers, None);
    assert_eq!(
        projected.request,
        ErrorRequest {
            method: "POST".to_owned(),
            url: "https://api.cloudflare.com".to_owned(),
        }
    );
    assert_eq!(projected.response.status_code, 429);
    assert_eq!(projected.response.request, projected.request);
    assert!(!projected.status_code_is_synthesized);
}
