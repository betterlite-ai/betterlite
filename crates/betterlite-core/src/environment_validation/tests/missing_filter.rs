use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ProviderIdentity;
use rstest::rstest;
use secrecy::SecretString;

use super::{
    EnvironmentCheck, EnvironmentRequest, Inference, missing, names, validate_environment,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Passed {
    Key,
    Base,
    Version,
}

fn filtered(provider: &str, passed: &[Passed]) -> EnvironmentCheck {
    let key = SecretString::from("sk-test");
    let request = EnvironmentRequest {
        model: Some("any-model"),
        api_key: passed.contains(&Passed::Key).then_some(&key),
        api_base: passed
            .contains(&Passed::Base)
            .then_some("https://example.test"),
        api_version: passed.contains(&Passed::Version).then_some("2024-01-01"),
    };
    let inference = Inference(Ok(ProviderIdentity::Custom(provider.to_owned())));
    validate_environment(
        &request,
        &names(&[]),
        &ProviderModelSets::default(),
        &inference,
    )
}

const NONE: &[Passed] = &[];
const KEY: &[Passed] = &[Passed::Key];
const BASE: &[Passed] = &[Passed::Base];
const VERSION: &[Passed] = &[Passed::Version];
const ALL: &[Passed] = &[Passed::Key, Passed::Base, Passed::Version];

#[rstest]
#[case::azure_nothing_passed("azure", NONE, &["AZURE_API_BASE", "AZURE_API_VERSION", "AZURE_API_KEY"])]
#[case::azure_key("azure", KEY, &["AZURE_API_BASE", "AZURE_API_VERSION"])]
#[case::azure_base("azure", BASE, &["AZURE_API_VERSION", "AZURE_API_KEY"])]
#[case::azure_version("azure", VERSION, &["AZURE_API_BASE", "AZURE_API_KEY"])]
#[case::azure_all("azure", ALL, &[])]
#[case::openai_key("openai", KEY, &[])]
#[case::openai_base_leaves_the_key_name("openai", BASE, &["OPENAI_API_KEY"])]
#[case::openai_version_leaves_the_key_name("openai", VERSION, &["OPENAI_API_KEY"])]
#[case::ollama_base("ollama", BASE, &[])]
#[case::ollama_key_leaves_the_base_name("ollama", KEY, &["OLLAMA_API_BASE"])]
#[case::cloudflare_key("cloudflare", KEY, &["CLOUDFLARE_API_BASE"])]
#[case::cloudflare_base("cloudflare", BASE, &["CLOUDFLARE_API_KEY"])]
#[case::bedrock_names_hold_no_api_key("bedrock", ALL, &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"])]
#[case::datarobot_token_is_not_an_api_key("datarobot", KEY, &["DATAROBOT_API_TOKEN"])]
#[case::vertex_names_hold_none_of_the_patterns("vertex_ai", ALL, &["VERTEXAI_PROJECT", "VERTEXAI_LOCATION"])]
#[case::gemini_key("gemini", KEY, &[])]
fn missing_filter_removes_names_by_substring(
    #[case] provider: &str,
    #[case] passed: &[Passed],
    #[case] expected: &[&'static str],
) {
    assert_eq!(filtered(provider, passed), missing(expected));
}

#[test]
fn missing_filter_with_no_model_still_gives_false_and_an_empty_list() {
    let key = SecretString::from("sk-test");
    let request = EnvironmentRequest {
        model: None,
        api_key: Some(&key),
        api_base: Some("https://example.test"),
        api_version: Some("2024-01-01"),
    };
    let inference = Inference(Err("no provider"));
    let answer = validate_environment(
        &request,
        &names(&[]),
        &ProviderModelSets::default(),
        &inference,
    );
    assert!(!answer.keys_in_environment);
    assert!(answer.missing_keys.is_empty());
}

#[test]
fn missing_filter_an_empty_api_base_string_still_counts_as_passed() {
    let request = EnvironmentRequest {
        model: Some("any-model"),
        api_key: None,
        api_base: Some(""),
        api_version: None,
    };
    let inference = Inference(Ok(ProviderIdentity::Custom("ollama".to_owned())));
    let answer = validate_environment(
        &request,
        &names(&[]),
        &ProviderModelSets::default(),
        &inference,
    );
    assert_eq!(answer, missing(&[]));
}
