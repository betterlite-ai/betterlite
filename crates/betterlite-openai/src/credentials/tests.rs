#![cfg(test)]
use super::{
    CredentialsError, DataResidency, ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials,
    get_openai_credentials, infer_openai_data_residency, is_openai_backed_api_base,
    resolve_openai_api_base,
};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use std::collections::BTreeMap;
fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}
#[rstest]
#[case::key_and_organization_precedence(0)] #[case::base_url_before_api_base(1)] #[case::empty_organization(2)] #[case::openai_host_boundary(3)] #[case::regional_data_residency(4)]
#[rustfmt::skip]
fn credentials_cases(#[case] case: u8) { match case {
0 => {
    let lookup = env(&[("OPENAI_API_KEY", "env-key"), ("OPENAI_ORGANIZATION", "env-org")]);
    let explicit = ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("explicit-key")), organization: Some(String::from("explicit-org")) };
    let globals = LiteLlmGlobals { api_base: None, api_key: Some(SecretString::from("global-key")), openai_key: Some(SecretString::from("global-openai-key")), organization: Some(String::from("global-org")) };
    let resolved = get_openai_credentials(explicit, globals, &lookup);
    assert_eq!(resolved.api_key.as_ref().map(ExposeSecret::expose_secret), Some("explicit-key"));
    assert_eq!(resolved.organization.as_deref(), Some("explicit-org"));
    let only_litellm = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals { api_base: None, api_key: Some(SecretString::from("global-key")), openai_key: Some(SecretString::from("global-openai-key")), organization: Some(String::from("global-org")) }, &lookup);
    assert_eq!(only_litellm.api_key.as_ref().map(ExposeSecret::expose_secret), Some("global-key"));
    assert_eq!(only_litellm.organization.as_deref(), Some("global-org"));
    let only_openai_key = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals { api_base: None, api_key: None, openai_key: Some(SecretString::from("global-openai-key")), organization: None }, &lookup);
    assert_eq!(only_openai_key.api_key.as_ref().map(ExposeSecret::expose_secret), Some("global-openai-key"));
    assert_eq!(only_openai_key.organization.as_deref(), Some("env-org"));
    let only_env = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &lookup);
    assert_eq!(only_env.api_key.as_ref().map(ExposeSecret::expose_secret), Some("env-key"));
    assert_eq!(only_env.organization.as_deref(), Some("env-org"));
    let nothing = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &BTreeMap::new());
    assert!(nothing.api_key.is_none() && nothing.organization.is_none());
},
1 => {
    let lookup = env(&[("OPENAI_BASE_URL", "https://base-url.example/v1?foo=bar"), ("OPENAI_API_BASE", "https://api-base.example/v1/")]);
    assert_eq!(resolve_openai_api_base(Some("https://explicit.example/v1"), Some("https://litellm.example/v1"), &lookup), "https://explicit.example/v1");
    assert_eq!(resolve_openai_api_base(None, Some("https://litellm.example/v1"), &lookup), "https://litellm.example/v1");
    assert_eq!(resolve_openai_api_base(None, None, &lookup), "https://base-url.example/v1?foo=bar");
    let api_base_only = env(&[("OPENAI_API_BASE", "https://api-base.example/v1/")]);
    assert_eq!(resolve_openai_api_base(None, None, &api_base_only), "https://api-base.example/v1/");
    assert_eq!(resolve_openai_api_base(None, None, &BTreeMap::new()), super::DEFAULT_OPENAI_API_BASE);
    let credentials = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &lookup);
    assert_eq!(credentials.api_base, "https://base-url.example/v1?foo=bar");
},
2 => {
    let lookup = env(&[("OPENAI_ORGANIZATION", "")]);
    let explicit_empty = ExplicitCredentials { api_base: None, api_key: None, organization: Some(String::new()) };
    let globals = LiteLlmGlobals { api_base: None, api_key: None, openai_key: None, organization: Some(String::from("global-org")) };
    let resolved = get_openai_credentials(explicit_empty, globals, &lookup);
    assert_eq!(resolved.organization.as_deref(), Some("global-org"));
    let empty_globals = LiteLlmGlobals { api_base: None, api_key: None, openai_key: None, organization: Some(String::new()) };
    let resolved_env_empty = get_openai_credentials(ExplicitCredentials::default(), empty_globals, &lookup);
    assert!(resolved_env_empty.organization.is_none());
},
3 => {
    assert!(is_openai_backed_api_base("https://api.openai.com/v1").expect("valid url"));
    assert!(is_openai_backed_api_base("http://eu.api.openai.com/v1").expect("valid url"));
    assert!(!is_openai_backed_api_base("https://fakeapi.openai.com").expect("valid url"));
    assert!(!is_openai_backed_api_base("https://api.openai.com.attacker.example").expect("valid url"));
    assert!(!is_openai_backed_api_base("mailto:ops@example.test").expect("valid url"));
    assert!(matches!(is_openai_backed_api_base("http://[::1"), Err(CredentialsError::InvalidApiBase(_))));
},
4 => {
    assert_eq!(infer_openai_data_residency(Some("openai"), Some("https://eu.api.openai.com/v1")), Some(DataResidency::Eu));
    assert_eq!(infer_openai_data_residency(Some("openai"), Some("https://US.API.OPENAI.COM:443/v1")), Some(DataResidency::Us));
    assert_eq!(infer_openai_data_residency(Some("azure"), Some("https://eu.api.openai.com/v1")), None);
    assert_eq!(infer_openai_data_residency(Some("openai"), Some("https://api.openai.com/v1")), None);
    assert_eq!(infer_openai_data_residency(Some("openai"), None), None);
    assert_eq!(infer_openai_data_residency(Some("openai"), Some("http://[::1")), None);
},
_ => panic!("unknown case {case}") } }
#[test]
#[rustfmt::skip]
fn openai_credentials_credential_identity_isolation() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let identity_a = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-a")), organization: Some(String::from("org-a")) }, LiteLlmGlobals::default(), &lookup);
    let identity_b = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-b")), organization: Some(String::from("org-b")) }, LiteLlmGlobals::default(), &lookup);
    assert_eq!(identity_a.organization.as_deref(), Some("org-a"));
    assert_eq!(identity_b.organization.as_deref(), Some("org-b"));
    assert_eq!(identity_a.api_key.as_ref().map(ExposeSecret::expose_secret), Some("key-a"));
    assert_eq!(identity_b.api_key.as_ref().map(ExposeSecret::expose_secret), Some("key-b"));
    let identity_a_again = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-a")), organization: Some(String::from("org-a")) }, LiteLlmGlobals::default(), &lookup);
    assert_eq!(identity_a_again.organization, identity_a.organization);
    assert_eq!(identity_a_again.api_key.as_ref().map(ExposeSecret::expose_secret), identity_a.api_key.as_ref().map(ExposeSecret::expose_secret));
}
#[test]
#[rustfmt::skip]
fn openai_credentials_secret_safe_failure() {
    let credentials = OpenAiCredentials { api_base: String::from(super::DEFAULT_OPENAI_API_BASE), api_key: Some(SecretString::from("fixture-secret")), organization: Some(String::from("fixture-org")) };
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("fixture-secret") && debug.contains("OpenAiCredentials"));
    let failure = is_openai_backed_api_base("http://[::1secret-fixture-token");
    let message = match failure { Err(error) => format!("{error:?}"), Ok(_) => panic!("expected an invalid url error") };
    assert!(!message.contains("secret-fixture-token"));
}
