#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::prompt_spec::PromptLiteLLMParams;
use rstest::rstest;

use super::{InitializerError, prompt_initializer};
use crate::arize::phoenix::tests::server::{PROMPT_BODY, proxied, serve};

fn params(
    api_key: Option<&str>,
    api_base: Option<&str>,
    prompt_id: Option<&str>,
) -> PromptLiteLLMParams {
    let fields = [
        ("prompt_integration", Some("arize_phoenix")),
        ("api_key", api_key),
        ("api_base", api_base),
        ("prompt_id", prompt_id),
    ];
    let body: BTreeMap<&str, &str> = fields
        .into_iter()
        .filter_map(|(name, value)| Some((name, value?)))
        .collect();
    serde_json::from_str(&serde_json::to_string(&body).unwrap()).unwrap()
}

fn environment(key: Option<&str>) -> BTreeMap<String, String> {
    key.map(|value| ("PHOENIX_API_KEY".to_owned(), value.to_owned()))
        .into_iter()
        .collect()
}

#[rstest]
#[case::no_key(None, Some("http://phoenix"), None)]
#[case::no_base(Some("k"), None, None)]
#[case::neither(None, None, None)]
#[case::empty_keys(Some(""), Some("http://phoenix"), Some(""))]
#[case::empty_base(Some("k"), Some(""), None)]
fn integration_initializer_requires_one_combined_message_for_key_and_base(
    #[case] api_key: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] env_key: Option<&str>,
) {
    let error = prompt_initializer(
        &params(api_key, api_base, None),
        &environment(env_key),
        proxied(),
    )
    .unwrap_err();
    assert!(matches!(error, InitializerError::MissingCredentials));
    assert_eq!(
        error.to_string(),
        "api_key and api_base are required for Arize Phoenix prompt integration"
    );
}

#[rstest]
#[case::configured(Some("v1"), Some("v1"))]
#[case::empty_counts_as_unset(Some(""), None)]
#[case::absent(None, None)]
fn integration_initializer_keeps_the_configured_prompt_id(
    #[case] configured: Option<&str>,
    #[case] kept: Option<&str>,
) {
    let manager = prompt_initializer(
        &params(Some("k"), Some("http://phoenix"), configured),
        &environment(None),
        proxied(),
    )
    .unwrap();
    assert_eq!(manager.prompt_id.as_deref(), kept);
}

#[tokio::test]
async fn integration_initializer_with_an_empty_prompt_id_fetches_only_the_requested_prompt() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let manager = prompt_initializer(
        &params(Some("k"), Some(&phoenix.base), Some("")),
        &environment(None),
        proxied(),
    )
    .unwrap();
    manager
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap();
    assert_eq!(phoenix.request_heads().len(), 1);
}

#[rstest]
#[case::explicit_key(
    Some("explicit"),
    Some("from-env"),
    "authorization: Bearer explicit\r\n"
)]
#[case::environment_key(None, Some("from-env"), "authorization: Bearer from-env\r\n")]
#[case::empty_explicit_key_falls_back(
    Some(""),
    Some("from-env"),
    "authorization: Bearer from-env\r\n"
)]
#[tokio::test]
async fn integration_initializer_sends_the_explicit_key_or_phoenix_api_key(
    #[case] api_key: Option<&str>,
    #[case] env_key: Option<&str>,
    #[case] header: &str,
) {
    let phoenix = serve(200, PROMPT_BODY).await;
    let manager = prompt_initializer(
        &params(api_key, Some(&phoenix.base), None),
        &environment(env_key),
        proxied(),
    )
    .unwrap();
    manager
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap();
    assert!(phoenix.request_heads()[0].contains(header));
}
