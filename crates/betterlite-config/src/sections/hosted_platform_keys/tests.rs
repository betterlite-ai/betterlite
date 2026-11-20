#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::hosted_platform_keys::{Headers, Settings};

#[derive(Clone, Copy, Debug)]
enum Setting {
    ApiBase,
    ApiKey,
    Headers,
    GroqKey,
    CloudflareKey,
    GroqApiKey,
    GroqApiBase,
    CloudflareApiKey,
    CloudflareApiBase,
    CloudflareAccountId,
    FireworksAiApiKey,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .hosted_platform_keys)
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
    match setting {
        Setting::ApiBase => settings.api_base.clone(),
        Setting::ApiKey => secret(&settings.api_key),
        Setting::Headers => settings.headers.as_ref().map(header_text),
        Setting::GroqKey => secret(&settings.groq_key),
        Setting::CloudflareKey => secret(&settings.cloudflare_api_key),
        Setting::GroqApiKey => secret(&settings.environment.groq_api_key),
        Setting::GroqApiBase => settings.environment.groq_api_base.clone(),
        Setting::CloudflareApiKey => secret(&settings.environment.cloudflare_api_key),
        Setting::CloudflareApiBase => settings.environment.cloudflare_api_base.clone(),
        Setting::CloudflareAccountId => settings.environment.cloudflare_account_id.clone(),
        Setting::FireworksAiApiKey => secret(&settings.environment.fireworks_ai_api_key),
    }
}

#[rstest]
#[case::loads_litellm_settings_api_base(Setting::ApiBase)]
#[case::loads_litellm_settings_api_key(Setting::ApiKey)]
#[case::loads_litellm_settings_headers(Setting::Headers)]
#[case::loads_litellm_settings_groq_key(Setting::GroqKey)]
#[case::loads_litellm_settings_cloudflare_api_key(Setting::CloudflareKey)]
#[case::loads_groq_api_key(Setting::GroqApiKey)]
#[case::loads_groq_api_base(Setting::GroqApiBase)]
#[case::loads_cloudflare_api_key(Setting::CloudflareApiKey)]
#[case::loads_cloudflare_api_base(Setting::CloudflareApiBase)]
#[case::loads_cloudflare_account_id(Setting::CloudflareAccountId)]
#[case::loads_fireworks_ai_api_key(Setting::FireworksAiApiKey)]
fn hosted_platform_keys_config_cases(#[case] setting: Setting) {
    assert_eq!(read(&load("", &[]).unwrap(), setting), None);
    match setting {
        Setting::ApiBase => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_base: http://global:4000\n", &[]).unwrap(),
                    setting
                ),
                Some(String::from("http://global:4000"))
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  api_base: os.environ/GLOBAL_BASE\n",
                        &[("GLOBAL_BASE", "http://from-env:4000")]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("http://from-env:4000"))
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_base: \"\"\n", &[]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::ApiKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_key: sk-global\n", &[]).unwrap(),
                    setting
                ),
                Some(String::from("sk-global"))
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  api_key: os.environ/GLOBAL_KEY\n",
                        &[("GLOBAL_KEY", "sk-from-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("sk-from-env"))
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::Headers => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  headers:\n    x-scope: tenant\n", &[]).unwrap(),
                    setting
                ),
                Some(String::from("x-scope=tenant"))
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  headers:\n    Authorization: os.environ/HEADER_KEY\n",
                        &[("HEADER_KEY", "Bearer sk-header")]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("Authorization=Bearer sk-header"))
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  headers: {}\n", &[]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::GroqKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  groq_key: sk-groq-global\n", &[]).unwrap(),
                    setting
                ),
                Some(String::from("sk-groq-global"))
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  groq_key: os.environ/GROQ_GLOBAL\n",
                        &[("GROQ_GLOBAL", "sk-groq-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("sk-groq-env"))
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  groq_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::CloudflareKey => {
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  cloudflare_api_key: sk-cloudflare-global\n",
                        &[]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("sk-cloudflare-global"))
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  cloudflare_api_key: os.environ/CLOUDFLARE_GLOBAL\n",
                        &[("CLOUDFLARE_GLOBAL", "sk-cloudflare-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("sk-cloudflare-env"))
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  cloudflare_api_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::GroqApiKey => {
            assert_eq!(
                read(&load("", &[("GROQ_API_KEY", "sk-groq")]).unwrap(), setting),
                Some(String::from("sk-groq"))
            );
            assert_eq!(
                read(&load("", &[("GROQ_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::GroqApiBase => {
            assert_eq!(
                read(
                    &load("", &[("GROQ_API_BASE", "http://groq:8000/openai/v1")]).unwrap(),
                    setting
                ),
                Some(String::from("http://groq:8000/openai/v1"))
            );
            assert_eq!(
                read(&load("", &[("GROQ_API_BASE", "")]).unwrap(), setting),
                None
            );
        }
        Setting::CloudflareApiKey => {
            assert_eq!(
                read(
                    &load("", &[("CLOUDFLARE_API_KEY", "sk-cloudflare")]).unwrap(),
                    setting
                ),
                Some(String::from("sk-cloudflare"))
            );
            assert_eq!(
                read(&load("", &[("CLOUDFLARE_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::CloudflareApiBase => {
            assert_eq!(
                read(
                    &load(
                        "",
                        &[(
                            "CLOUDFLARE_API_BASE",
                            "https://gateway.ai.cloudflare.com/v1"
                        )]
                    )
                    .unwrap(),
                    setting
                ),
                Some(String::from("https://gateway.ai.cloudflare.com/v1"))
            );
            assert_eq!(
                read(&load("", &[("CLOUDFLARE_API_BASE", "")]).unwrap(), setting),
                None
            );
        }
        Setting::CloudflareAccountId => {
            assert_eq!(
                read(
                    &load("", &[("CLOUDFLARE_ACCOUNT_ID", "account-1234")]).unwrap(),
                    setting
                ),
                Some(String::from("account-1234"))
            );
            assert_eq!(
                read(
                    &load("", &[("CLOUDFLARE_ACCOUNT_ID", "")]).unwrap(),
                    setting
                ),
                None
            );
        }
        Setting::FireworksAiApiKey => {
            assert_eq!(
                read(
                    &load("", &[("FIREWORKS_AI_API_KEY", "sk-fireworks")]).unwrap(),
                    setting
                ),
                Some(String::from("sk-fireworks"))
            );
            assert_eq!(
                read(&load("", &[("FIREWORKS_AI_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
    }
    let unrelated = match setting {
        Setting::ApiBase => "litellm_settings:\n  api_base_url: http://global\n",
        Setting::ApiKey => "litellm_settings:\n  api_token: sk-unrelated\n",
        Setting::Headers => "litellm_settings:\n  headers_extra:\n    x-scope: tenant\n",
        Setting::GroqKey => "litellm_settings:\n  groq_global_key: sk-unrelated\n",
        Setting::CloudflareKey => "litellm_settings:\n  cloudflare_global_key: sk-unrelated\n",
        Setting::GroqApiKey => "litellm_settings:\n  groq_api_key: sk-unrelated\n",
        Setting::GroqApiBase => "litellm_settings:\n  groq_api_base: http://unrelated\n",
        Setting::CloudflareApiKey => "litellm_settings:\n  cloudflare_key: sk-unrelated\n",
        Setting::CloudflareApiBase => "litellm_settings:\n  cloudflare_base: http://unrelated\n",
        Setting::CloudflareAccountId => "litellm_settings:\n  cloudflare_account: unrelated\n",
        Setting::FireworksAiApiKey => "litellm_settings:\n  fireworks_key: sk-unrelated\n",
    };
    assert!(matches!(
        load(unrelated, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}

// the provider variables sit beside the SDK globals, and each keeps its own value
#[rstest]
#[case::api_base(Setting::ApiBase, "http://global:4000")]
#[case::api_key(Setting::ApiKey, "sk-global")]
#[case::headers(Setting::Headers, "x-scope=tenant")]
#[case::groq_key(Setting::GroqKey, "sk-groq-global")]
#[case::cloudflare_key(Setting::CloudflareKey, "sk-cloudflare-global")]
#[case::groq_api_key(Setting::GroqApiKey, "sk-groq-env")]
#[case::groq_api_base(Setting::GroqApiBase, "http://groq:8000/openai/v1")]
#[case::cloudflare_api_key(Setting::CloudflareApiKey, "sk-cloudflare-env")]
#[case::cloudflare_api_base(Setting::CloudflareApiBase, "https://gateway.ai.cloudflare.com/v1")]
#[case::cloudflare_account_id(Setting::CloudflareAccountId, "account-1234")]
#[case::fireworks_ai_api_key(Setting::FireworksAiApiKey, "sk-fireworks-env")]
fn hosted_platform_keys_reads_every_setting_at_once(#[case] setting: Setting, #[case] value: &str) {
    let settings = load(
        "litellm_settings:\n  api_base: http://global:4000\n  api_key: sk-global\n  groq_key: sk-groq-global\n  cloudflare_api_key: sk-cloudflare-global\n  headers:\n    x-scope: tenant\n",
        &[
            ("GROQ_API_KEY", "sk-groq-env"),
            ("GROQ_API_BASE", "http://groq:8000/openai/v1"),
            ("CLOUDFLARE_API_KEY", "sk-cloudflare-env"),
            ("CLOUDFLARE_API_BASE", "https://gateway.ai.cloudflare.com/v1"),
            ("CLOUDFLARE_ACCOUNT_ID", "account-1234"),
            ("FIREWORKS_AI_API_KEY", "sk-fireworks-env"),
        ],
    )
    .unwrap();
    assert_eq!(read(&settings, setting), Some(String::from(value)));
}
