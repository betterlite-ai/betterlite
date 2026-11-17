#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::self_hosted_dispatch::Settings;

#[derive(Clone, Copy, Debug)]
enum Setting {
    HostedVllmApiBase,
    HostedVllmApiKey,
    OllamaApiBase,
    OllamaApiKey,
    LemonadeApiKey,
    ApiBase,
    ApiKey,
    OllamaKey,
    LemonadeKey,
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .self_hosted_dispatch)
}

fn read(settings: &Settings, setting: Setting) -> Option<String> {
    let secret = |value: &Option<secrecy::SecretString>| {
        value.as_ref().map(|key| key.expose_secret().to_owned())
    };
    match setting {
        Setting::HostedVllmApiBase => settings.hosted_vllm_api_base.clone(),
        Setting::HostedVllmApiKey => secret(&settings.hosted_vllm_api_key),
        Setting::OllamaApiBase => settings.ollama_api_base.clone(),
        Setting::OllamaApiKey => secret(&settings.ollama_api_key),
        Setting::LemonadeApiKey => secret(&settings.lemonade_api_key),
        Setting::ApiBase => settings.api_base.clone(),
        Setting::ApiKey => secret(&settings.api_key),
        Setting::OllamaKey => secret(&settings.ollama_key),
        Setting::LemonadeKey => secret(&settings.lemonade_key),
    }
}

#[rstest]
#[case::loads_hosted_vllm_api_base(Setting::HostedVllmApiBase)]
#[case::loads_hosted_vllm_api_key(Setting::HostedVllmApiKey)]
#[case::loads_ollama_api_base(Setting::OllamaApiBase)]
#[case::loads_ollama_api_key(Setting::OllamaApiKey)]
#[case::loads_lemonade_api_key(Setting::LemonadeApiKey)]
#[case::loads_litellm_settings_api_base(Setting::ApiBase)]
#[case::loads_litellm_settings_api_key(Setting::ApiKey)]
#[case::loads_litellm_settings_ollama_key(Setting::OllamaKey)]
#[case::loads_litellm_settings_lemonade_key(Setting::LemonadeKey)]
fn self_hosted_dispatch_config_cases(#[case] setting: Setting) {
    assert_eq!(read(&load("", &[]).unwrap(), setting), None);
    match setting {
        Setting::HostedVllmApiBase => {
            assert_eq!(
                read(
                    &load("", &[("HOSTED_VLLM_API_BASE", "http://vllm:8000/v1")]).unwrap(),
                    setting
                ),
                Some("http://vllm:8000/v1".to_owned())
            );
            assert_eq!(
                read(&load("", &[("HOSTED_VLLM_API_BASE", "")]).unwrap(), setting),
                None
            );
        }
        Setting::HostedVllmApiKey => {
            assert_eq!(
                read(
                    &load("", &[("HOSTED_VLLM_API_KEY", "sk-vllm")]).unwrap(),
                    setting
                ),
                Some("sk-vllm".to_owned())
            );
            assert_eq!(
                read(&load("", &[("HOSTED_VLLM_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::OllamaApiBase => {
            assert_eq!(
                read(
                    &load("", &[("OLLAMA_API_BASE", "http://ollama:11434")]).unwrap(),
                    setting
                ),
                Some("http://ollama:11434".to_owned())
            );
            assert_eq!(
                read(&load("", &[("OLLAMA_API_BASE", "")]).unwrap(), setting),
                None
            );
        }
        Setting::OllamaApiKey => {
            assert_eq!(
                read(
                    &load("", &[("OLLAMA_API_KEY", "sk-ollama")]).unwrap(),
                    setting
                ),
                Some("sk-ollama".to_owned())
            );
            assert_eq!(
                read(&load("", &[("OLLAMA_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::LemonadeApiKey => {
            assert_eq!(
                read(
                    &load("", &[("LEMONADE_API_KEY", "sk-lemonade")]).unwrap(),
                    setting
                ),
                Some("sk-lemonade".to_owned())
            );
            assert_eq!(
                read(&load("", &[("LEMONADE_API_KEY", "")]).unwrap(), setting),
                None
            );
        }
        Setting::ApiBase => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_base: http://global:4000\n", &[]).unwrap(),
                    setting
                ),
                Some("http://global:4000".to_owned())
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
                Some("http://from-env:4000".to_owned())
            );
            assert!(matches!(
                load("litellm_settings:\n  api_base: os.environ/MISSING\n", &[]),
                Err(ConfigError::MissingEnvironment { .. })
            ));
        }
        Setting::ApiKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_key: sk-global\n", &[]).unwrap(),
                    setting
                ),
                Some("sk-global".to_owned())
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
                Some("sk-from-env".to_owned())
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  api_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                Some(String::new())
            );
        }
        Setting::OllamaKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  ollama_key: sk-ollama\n", &[]).unwrap(),
                    setting
                ),
                Some("sk-ollama".to_owned())
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  ollama_key: os.environ/OLLAMA_GLOBAL\n",
                        &[("OLLAMA_GLOBAL", "sk-from-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some("sk-from-env".to_owned())
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  ollama_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                Some(String::new())
            );
        }
        Setting::LemonadeKey => {
            assert_eq!(
                read(
                    &load("litellm_settings:\n  lemonade_key: sk-lemonade\n", &[]).unwrap(),
                    setting
                ),
                Some("sk-lemonade".to_owned())
            );
            assert_eq!(
                read(
                    &load(
                        "litellm_settings:\n  lemonade_key: os.environ/LEMONADE_GLOBAL\n",
                        &[("LEMONADE_GLOBAL", "sk-from-env")]
                    )
                    .unwrap(),
                    setting
                ),
                Some("sk-from-env".to_owned())
            );
            assert_eq!(
                read(
                    &load("litellm_settings:\n  lemonade_key: \"\"\n", &[]).unwrap(),
                    setting
                ),
                Some(String::new())
            );
        }
    }
    let unrelated = match setting {
        Setting::HostedVllmApiBase => "litellm_settings:\n  hosted_vllm_api_base: http://vllm\n",
        Setting::HostedVllmApiKey => "litellm_settings:\n  hosted_vllm_api_key: sk-unrelated\n",
        Setting::OllamaApiBase => "litellm_settings:\n  ollama_api_base: http://ollama\n",
        Setting::OllamaApiKey => "litellm_settings:\n  ollama_api_key: sk-unrelated\n",
        Setting::LemonadeApiKey => "litellm_settings:\n  lemonade_api_key: sk-unrelated\n",
        Setting::ApiBase => "litellm_settings:\n  api_base_url: http://global\n",
        Setting::ApiKey => "litellm_settings:\n  api_token: sk-unrelated\n",
        Setting::OllamaKey => "litellm_settings:\n  ollama_global_key: sk-unrelated\n",
        Setting::LemonadeKey => "litellm_settings:\n  lemonade_global_key: sk-unrelated\n",
    };
    assert!(matches!(
        load(unrelated, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}

// the five provider variables live beside the SDK globals, and each keeps its own value
#[test]
fn self_hosted_dispatch_reads_every_setting_at_once() {
    let settings = load(
        "litellm_settings:\n  api_base: http://global:4000\n  api_key: sk-global\n  ollama_key: sk-ollama-global\n  lemonade_key: sk-lemonade-global\n",
        &[
            ("HOSTED_VLLM_API_BASE", "http://vllm:8000/v1"),
            ("HOSTED_VLLM_API_KEY", "sk-vllm-env"),
            ("OLLAMA_API_BASE", "http://ollama:11434"),
            ("OLLAMA_API_KEY", "sk-ollama-env"),
            ("LEMONADE_API_KEY", "sk-lemonade-env"),
        ],
    )
    .unwrap();
    assert_eq!(
        read(&settings, Setting::HostedVllmApiBase),
        Some("http://vllm:8000/v1".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::HostedVllmApiKey),
        Some("sk-vllm-env".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::OllamaApiBase),
        Some("http://ollama:11434".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::OllamaApiKey),
        Some("sk-ollama-env".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::LemonadeApiKey),
        Some("sk-lemonade-env".to_owned())
    );
}

#[test]
fn self_hosted_dispatch_reads_every_sdk_global() {
    let settings = load(
        "litellm_settings:\n  api_base: http://global:4000\n  api_key: sk-global\n  ollama_key: sk-ollama-global\n  lemonade_key: sk-lemonade-global\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        read(&settings, Setting::ApiBase),
        Some("http://global:4000".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::ApiKey),
        Some("sk-global".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::OllamaKey),
        Some("sk-ollama-global".to_owned())
    );
    assert_eq!(
        read(&settings, Setting::LemonadeKey),
        Some("sk-lemonade-global".to_owned())
    );
}
