#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::gateway_provider_keys::{Headers, Settings};

#[derive(Clone, Copy, Debug)]
enum Setting {
    ApiBase,
    ApiKey,
    Headers,
    OpenrouterKey,
    OpenrouterApiBase,
    OpenrouterApiKey,
    OrApiKey,
    OrSiteUrl,
    OrAppName,
    VercelAiGatewayApiBase,
    VercelAiGatewayApiKey,
    VercelOidcToken,
    VercelSiteUrl,
    VercelAppName,
}

const GLOBALS: &str = "litellm_settings:\n  api_base: http://global:4000\n  api_key: sk-global\n  openrouter_key: sk-openrouter-global\n  headers:\n    x-scope: tenant\n";

const EMPTY_GLOBALS: &str =
    "litellm_settings:\n  api_base: \"\"\n  api_key: \"\"\n  openrouter_key: \"\"\n  headers: {}\n";

const ENVIRONMENT: [(&str, &str); 10] = [
    ("OPENROUTER_API_BASE", "http://openrouter:8000/api/v1"),
    ("OPENROUTER_API_KEY", "sk-openrouter-env"),
    ("OR_API_KEY", "sk-or-env"),
    ("OR_SITE_URL", "https://or.example"),
    ("OR_APP_NAME", "or-app"),
    ("VERCEL_AI_GATEWAY_API_BASE", "http://vercel:8000/v1"),
    ("VERCEL_AI_GATEWAY_API_KEY", "sk-vercel-env"),
    ("VERCEL_OIDC_TOKEN", "oidc-env"),
    ("VERCEL_SITE_URL", "https://vercel.example"),
    ("VERCEL_APP_NAME", "vercel-app"),
];

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .gateway_provider_keys)
}

fn header_text(headers: &Headers) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn read(settings: &Settings, setting: Setting) -> Option<String> {
    let secret =
        |value: &Option<SecretString>| value.as_ref().map(|key| key.expose_secret().to_owned());
    let environment = &settings.environment;
    match setting {
        Setting::ApiBase => settings.api_base.clone(),
        Setting::ApiKey => secret(&settings.api_key),
        Setting::Headers => settings.headers.as_ref().map(header_text),
        Setting::OpenrouterKey => secret(&settings.openrouter_key),
        Setting::OpenrouterApiBase => environment.openrouter_api_base.clone(),
        Setting::OpenrouterApiKey => secret(&environment.openrouter_api_key),
        Setting::OrApiKey => secret(&environment.or_api_key),
        Setting::OrSiteUrl => environment.or_site_url.clone(),
        Setting::OrAppName => environment.or_app_name.clone(),
        Setting::VercelAiGatewayApiBase => environment.vercel_ai_gateway_api_base.clone(),
        Setting::VercelAiGatewayApiKey => secret(&environment.vercel_ai_gateway_api_key),
        Setting::VercelOidcToken => secret(&environment.vercel_oidc_token),
        Setting::VercelSiteUrl => environment.vercel_site_url.clone(),
        Setting::VercelAppName => environment.vercel_app_name.clone(),
    }
}

#[rstest]
#[case::litellm_settings_api_base(Setting::ApiBase)]
#[case::litellm_settings_api_key(Setting::ApiKey)]
#[case::litellm_settings_headers(Setting::Headers)]
#[case::litellm_settings_openrouter_key(Setting::OpenrouterKey)]
#[case::openrouter_api_base(Setting::OpenrouterApiBase)]
#[case::openrouter_api_key(Setting::OpenrouterApiKey)]
#[case::or_api_key(Setting::OrApiKey)]
#[case::or_site_url(Setting::OrSiteUrl)]
#[case::or_app_name(Setting::OrAppName)]
#[case::vercel_ai_gateway_api_base(Setting::VercelAiGatewayApiBase)]
#[case::vercel_ai_gateway_api_key(Setting::VercelAiGatewayApiKey)]
#[case::vercel_oidc_token(Setting::VercelOidcToken)]
#[case::vercel_site_url(Setting::VercelSiteUrl)]
#[case::vercel_app_name(Setting::VercelAppName)]
fn gateway_provider_keys_default_to_none(#[case] setting: Setting) {
    assert_eq!(read(&load("", &[]).unwrap(), setting), None);
}

#[rstest]
#[case::api_base(Setting::ApiBase, "http://global:4000")]
#[case::api_key(Setting::ApiKey, "sk-global")]
#[case::headers(Setting::Headers, "x-scope=tenant")]
#[case::openrouter_key(Setting::OpenrouterKey, "sk-openrouter-global")]
#[case::openrouter_api_base(Setting::OpenrouterApiBase, "http://openrouter:8000/api/v1")]
#[case::openrouter_api_key(Setting::OpenrouterApiKey, "sk-openrouter-env")]
#[case::or_api_key(Setting::OrApiKey, "sk-or-env")]
#[case::or_site_url(Setting::OrSiteUrl, "https://or.example")]
#[case::or_app_name(Setting::OrAppName, "or-app")]
#[case::vercel_ai_gateway_api_base(Setting::VercelAiGatewayApiBase, "http://vercel:8000/v1")]
#[case::vercel_ai_gateway_api_key(Setting::VercelAiGatewayApiKey, "sk-vercel-env")]
#[case::vercel_oidc_token(Setting::VercelOidcToken, "oidc-env")]
#[case::vercel_site_url(Setting::VercelSiteUrl, "https://vercel.example")]
#[case::vercel_app_name(Setting::VercelAppName, "vercel-app")]
fn gateway_provider_keys_reads_every_setting_at_once(
    #[case] setting: Setting,
    #[case] value: &str,
) {
    let settings = load(GLOBALS, &ENVIRONMENT).unwrap();
    assert_eq!(read(&settings, setting), Some(String::from(value)));
}

#[rstest]
#[case::api_base(Setting::ApiBase)]
#[case::api_key(Setting::ApiKey)]
#[case::headers(Setting::Headers)]
#[case::openrouter_key(Setting::OpenrouterKey)]
#[case::openrouter_api_base(Setting::OpenrouterApiBase)]
#[case::openrouter_api_key(Setting::OpenrouterApiKey)]
#[case::or_api_key(Setting::OrApiKey)]
#[case::or_site_url(Setting::OrSiteUrl)]
#[case::or_app_name(Setting::OrAppName)]
#[case::vercel_ai_gateway_api_base(Setting::VercelAiGatewayApiBase)]
#[case::vercel_ai_gateway_api_key(Setting::VercelAiGatewayApiKey)]
#[case::vercel_oidc_token(Setting::VercelOidcToken)]
#[case::vercel_site_url(Setting::VercelSiteUrl)]
#[case::vercel_app_name(Setting::VercelAppName)]
fn gateway_provider_keys_count_an_empty_value_as_unset(#[case] setting: Setting) {
    let empty: Vec<(&str, &str)> = ENVIRONMENT.iter().map(|(name, _)| (*name, "")).collect();
    assert_eq!(read(&load(EMPTY_GLOBALS, &empty).unwrap(), setting), None);
}

// the four litellm_settings values accept the os.environ/ form like every other section
#[test]
fn gateway_provider_keys_resolve_os_environ_references() {
    let settings = load(
        "litellm_settings:\n  api_base: os.environ/GLOBAL_BASE\n  api_key: os.environ/GLOBAL_KEY\n  openrouter_key: os.environ/OPENROUTER_GLOBAL\n  headers:\n    Authorization: os.environ/HEADER_KEY\n",
        &[
            ("GLOBAL_BASE", "http://from-env:4000"),
            ("GLOBAL_KEY", "sk-from-env"),
            ("OPENROUTER_GLOBAL", "sk-openrouter-env"),
            ("HEADER_KEY", "Bearer sk-header"),
        ],
    )
    .unwrap();
    assert_eq!(settings.api_base.as_deref(), Some("http://from-env:4000"));
    assert_eq!(
        read(&settings, Setting::ApiKey).as_deref(),
        Some("sk-from-env")
    );
    assert_eq!(
        read(&settings, Setting::OpenrouterKey).as_deref(),
        Some("sk-openrouter-env")
    );
    assert_eq!(
        settings.headers.as_ref().map(header_text).as_deref(),
        Some("Authorization=Bearer sk-header")
    );
}

// a key beside the ones this section reads stays unsupported until a section reads it
#[rstest]
#[case::api_base("litellm_settings:\n  gateway_api_base: http://unrelated\n")]
#[case::api_key("litellm_settings:\n  gateway_api_key: sk-unrelated\n")]
#[case::headers("litellm_settings:\n  gateway_headers:\n    x-scope: tenant\n")]
#[case::openrouter_key("litellm_settings:\n  openrouter_global_key: sk-unrelated\n")]
fn gateway_provider_keys_reject_unread_keys(#[case] source: &str) {
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}
