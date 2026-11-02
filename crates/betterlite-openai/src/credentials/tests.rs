#![cfg(test)]
use super::{
    CredentialsError, DataResidency, ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials,
    get_openai_credentials, infer_openai_data_residency, is_openai_backed_api_base,
    resolve_openai_api_base,
};
use betterlite_config::sections::openai_credentials::{DEFAULT_API_BASE, Settings};
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

fn settings(api_base: &str, organization: Option<&str>) -> Settings {
    Settings {
        api_base: api_base.to_owned(),
        organization: organization.map(str::to_owned),
    }
}

#[rstest]
#[case::key_and_organization_precedence(0)] #[case::base_url_before_api_base(1)] #[case::empty_organization(2)] #[case::openai_host_boundary(3)] #[case::regional_data_residency(4)]
#[rustfmt::skip]
fn credentials_cases(#[case] case: u8) { match case {
0 => {
    let env_settings = settings(DEFAULT_API_BASE, Some("env-org"));
    let env_api_key = Some(SecretString::from("env-key"));
    let explicit = ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("explicit-key")), organization: Some(String::from("explicit-org")) };
    let globals = LiteLlmGlobals { api_base: None, api_key: Some(SecretString::from("global-key")), openai_key: Some(SecretString::from("global-openai-key")), organization: Some(String::from("global-org")) };
    let resolved = get_openai_credentials(explicit, globals, &env_settings, env_api_key.clone());
    assert_eq!(resolved.api_key.as_ref().map(ExposeSecret::expose_secret), Some("explicit-key"));
    assert_eq!(resolved.organization.as_deref(), Some("explicit-org"));
    let only_litellm = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals { api_base: None, api_key: Some(SecretString::from("global-key")), openai_key: Some(SecretString::from("global-openai-key")), organization: Some(String::from("global-org")) }, &env_settings, env_api_key.clone());
    assert_eq!(only_litellm.api_key.as_ref().map(ExposeSecret::expose_secret), Some("global-key"));
    assert_eq!(only_litellm.organization.as_deref(), Some("global-org"));
    let only_openai_key = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals { api_base: None, api_key: None, openai_key: Some(SecretString::from("global-openai-key")), organization: None }, &env_settings, env_api_key.clone());
    assert_eq!(only_openai_key.api_key.as_ref().map(ExposeSecret::expose_secret), Some("global-openai-key"));
    assert_eq!(only_openai_key.organization.as_deref(), Some("env-org"));
    let only_env = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &env_settings, env_api_key);
    assert_eq!(only_env.api_key.as_ref().map(ExposeSecret::expose_secret), Some("env-key"));
    assert_eq!(only_env.organization.as_deref(), Some("env-org"));
    let empty_settings = Settings::default();
    let nothing = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &empty_settings, None);
    assert!(nothing.api_key.is_none() && nothing.organization.is_none());
    let empty_explicit_key = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("")), organization: None }, LiteLlmGlobals { api_base: None, api_key: Some(SecretString::from("global-key")), openai_key: None, organization: None }, &empty_settings, None);
    assert_eq!(empty_explicit_key.api_key.as_ref().map(ExposeSecret::expose_secret), Some("global-key"));
},
1 => {
    let base_url_settings = settings("https://base-url.example/v1?foo=bar", None);
    assert_eq!(resolve_openai_api_base(Some("https://explicit.example/v1"), Some("https://litellm.example/v1"), &base_url_settings), "https://explicit.example/v1");
    assert_eq!(resolve_openai_api_base(None, Some("https://litellm.example/v1"), &base_url_settings), "https://litellm.example/v1");
    assert_eq!(resolve_openai_api_base(None, None, &base_url_settings), "https://base-url.example/v1?foo=bar");
    let api_base_only = settings("https://api-base.example/v1/", None);
    assert_eq!(resolve_openai_api_base(None, None, &api_base_only), "https://api-base.example/v1/");
    assert_eq!(resolve_openai_api_base(None, None, &Settings::default()), DEFAULT_API_BASE);
    let credentials = get_openai_credentials(ExplicitCredentials::default(), LiteLlmGlobals::default(), &base_url_settings, None);
    assert_eq!(credentials.api_base, "https://base-url.example/v1?foo=bar");
},
2 => {
    let env_settings = settings(DEFAULT_API_BASE, None);
    let explicit_empty = ExplicitCredentials { api_base: None, api_key: None, organization: Some(String::new()) };
    let globals = LiteLlmGlobals { api_base: None, api_key: None, openai_key: None, organization: Some(String::from("global-org")) };
    let resolved = get_openai_credentials(explicit_empty, globals, &env_settings, None);
    assert_eq!(resolved.organization.as_deref(), Some("global-org"));
    let empty_globals = LiteLlmGlobals { api_base: None, api_key: None, openai_key: None, organization: Some(String::new()) };
    let resolved_env_empty = get_openai_credentials(ExplicitCredentials::default(), empty_globals, &env_settings, None);
    assert!(resolved_env_empty.organization.is_none());
},
3 => {
    assert!(is_openai_backed_api_base("https://api.openai.com/v1").expect("valid url"));
    assert!(is_openai_backed_api_base("http://eu.api.openai.com/v1").expect("valid url"));
    assert!(!is_openai_backed_api_base("https://fakeapi.openai.com").expect("valid url"));
    assert!(!is_openai_backed_api_base("https://api.openai.com.attacker.example").expect("valid url"));
    assert!(!is_openai_backed_api_base("mailto:ops@example.test").expect("valid url"));
    assert!(!is_openai_backed_api_base("https://").expect("missing host, not an error"));
    assert!(!is_openai_backed_api_base("api.openai.com/v1").expect("scheme-less, not an error"));
    assert!(is_openai_backed_api_base("https://api.openai.com:99999/v1").expect("out-of-range port, host still extracted"));
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
    let settings = Settings::default();
    let identity_a = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-a")), organization: Some(String::from("org-a")) }, LiteLlmGlobals::default(), &settings, None);
    let identity_b = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-b")), organization: Some(String::from("org-b")) }, LiteLlmGlobals::default(), &settings, None);
    assert_eq!(identity_a.organization.as_deref(), Some("org-a"));
    assert_eq!(identity_b.organization.as_deref(), Some("org-b"));
    assert_eq!(identity_a.api_key.as_ref().map(ExposeSecret::expose_secret), Some("key-a"));
    assert_eq!(identity_b.api_key.as_ref().map(ExposeSecret::expose_secret), Some("key-b"));
    let identity_a_again = get_openai_credentials(ExplicitCredentials { api_base: None, api_key: Some(SecretString::from("key-a")), organization: Some(String::from("org-a")) }, LiteLlmGlobals::default(), &settings, None);
    assert_eq!(identity_a_again.organization, identity_a.organization);
    assert_eq!(identity_a_again.api_key.as_ref().map(ExposeSecret::expose_secret), identity_a.api_key.as_ref().map(ExposeSecret::expose_secret));
}
#[test]
#[rustfmt::skip]
fn openai_credentials_secret_safe_failure() {
    let credentials = OpenAiCredentials { api_base: String::from(DEFAULT_API_BASE), api_key: Some(SecretString::from("fixture-secret")), organization: Some(String::from("fixture-org")) };
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("fixture-secret") && debug.contains("OpenAiCredentials"));
    let failure = is_openai_backed_api_base("http://[::1secret-fixture-token");
    let message = match failure { Err(error) => format!("{error:?}"), Ok(_) => panic!("expected an invalid url error") };
    assert!(!message.contains("secret-fixture-token"));
}
