use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use crate::credentials::AnthropicSettings;
use crate::headers::AnthropicHeaders;
use crate::headers::environment::{EnvironmentRequest, HeaderParams, validate_environment};
use crate::headers::request::{HeaderFeature, get_anthropic_headers};
use crate::headers::tests::fixture;

fn validated(
    settings: &AnthropicSettings,
    caller_headers: &IndexMap<String, String>,
    messages: &[Json],
) -> AnthropicHeaders {
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    validate_environment(
        &EnvironmentRequest {
            model: "claude-sonnet-4",
            caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages,
            optional_params: &params,
            effort_used: false,
        },
        settings,
    )
    .unwrap()
}

#[test]
fn anthropic_headers_credential_identity_isolation() {
    let first_settings = fixture::settings(Some("sk-ant-api03-first-identity"), None);
    let second_settings = fixture::settings(Some("sk-ant-api03-second-identity"), None);
    let messages = Vec::new();
    let empty = IndexMap::new();

    let first = validated(&first_settings, &empty, &messages);
    let second = validated(&second_settings, &empty, &messages);
    assert_eq!(
        fixture::header(&first, "x-api-key"),
        Some("sk-ant-api03-first-identity")
    );
    assert_eq!(
        fixture::header(&second, "x-api-key"),
        Some("sk-ant-api03-second-identity")
    );
    assert!(
        first
            .values
            .values()
            .all(|value| !value.contains("second-identity"))
    );
    assert!(
        second
            .values
            .values()
            .all(|value| !value.contains("first-identity"))
    );
    assert_ne!(first, second);

    // one identity's caller betas never reach another identity's request
    let caller_headers = fixture::headers(&[("anthropic-beta", "first-identity-beta")]);
    let with_beta = validated(&first_settings, &caller_headers, &messages);
    assert_eq!(
        fixture::beta_tokens(&with_beta),
        BTreeSet::from(["first-identity-beta"])
    );
    let other = validated(&second_settings, &empty, &messages);
    assert!(!other.values.contains_key("anthropic-beta"));

    // no cache keeps the earlier request's headers between identities
    let repeated = validated(&first_settings, &empty, &messages);
    assert_eq!(repeated, first);
}

#[test]
fn anthropic_headers_secret_safe_failure() {
    let features = fixture::features(&[HeaderFeature::FileId]);
    let api_key = fixture::secret("sk-ant-api03-fixture-secret");
    let headers = get_anthropic_headers(&fixture::request(Some(&api_key), None, &features));
    let debug = format!("{headers:?}");
    assert!(!debug.contains("fixture-secret"));
    assert!(debug.contains("x-api-key"));

    let oauth_key = fixture::secret("sk-ant-oat-fixture-secret");
    let oauth_headers = get_anthropic_headers(&fixture::request(Some(&oauth_key), None, &features));
    let debug = format!("{oauth_headers:?}");
    assert!(!debug.contains("fixture-secret"));
    assert!(debug.contains("authorization"));

    let token = fixture::secret("auth-token-fixture-secret");
    let token_headers = get_anthropic_headers(&fixture::request(None, Some(&token), &features));
    let debug = format!("{token_headers:?}");
    assert!(!debug.contains("fixture-secret"));

    // a missing-credential failure names the variables and never the caller's headers
    let caller_headers = fixture::headers(&[("x-api-key", "stale-fixture-secret")]);
    let params = fixture::params();
    let litellm_params = fixture::litellm_params();
    let settings = fixture::settings(None, None);
    let messages = Vec::new();
    let error = validate_environment(
        &EnvironmentRequest {
            model: "claude-sonnet-4",
            caller_headers: &caller_headers,
            api_key: None,
            api_base: None,
            litellm_params: &litellm_params,
            messages: &messages,
            optional_params: &params,
            effort_used: false,
        },
        &settings,
    )
    .unwrap_err();
    assert!(!format!("{error:?}").contains("stale-fixture-secret"));
    assert!(!error.to_string().contains("stale-fixture-secret"));
}
