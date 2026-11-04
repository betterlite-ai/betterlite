#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::loader::ConfigError;
use crate::loader::EnvironmentLookup;
use crate::sections::Section as _;
use crate::sections::huggingface_hub::{OfflineSetting, Raw, Settings, TransferSetting};
use secrecy::ExposeSecret as _;

struct TestEnvironment {
    values: BTreeMap<String, String>,
    home: Option<PathBuf>,
}

impl EnvironmentLookup for TestEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

fn settings(entries: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let variables = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Raw::default().convert(&TestEnvironment {
        values: variables,
        home: Some(PathBuf::from("/system/home")),
    })
}

#[test]
fn settings_follow_huggingface_hub() {
    defaults_follow_hub();
    configured_paths_follow_hub();
    staging_follows_hub();
    fallback_values_follow_hub();
    invalid_values_name_variables();
}

fn defaults_follow_hub() {
    let defaults = settings(&[]).unwrap();
    assert_eq!(defaults.endpoint, "https://huggingface.co");
    assert_eq!(defaults.offline, OfflineSetting::Online);
    assert_eq!(defaults.etag_timeout, 10);
    assert_eq!(defaults.download_timeout, 10);
    assert!(!defaults.token.disable_implicit);
    assert_eq!(
        defaults.hf_home,
        PathBuf::from("/system/home/.cache/huggingface")
    );
    assert_eq!(defaults.hub_cache, defaults.hf_home.join("hub"));
    assert_eq!(defaults.token_path, defaults.hf_home.join("token"));
}

fn configured_paths_follow_hub() {
    let configured = settings(&[
        ("HF_ENDPOINT", "https://hub.example///"),
        ("HF_HOME", "~/hf-$SUFFIX/${MISSING}"),
        ("HOME", "/home/test/"),
        ("SUFFIX", "cache"),
        ("HF_HUB_CACHE", "$SUFFIX/hub"),
        ("HF_TOKEN_PATH", "~/keys/token"),
        ("HF_HUB_ETAG_TIMEOUT", "0"),
        ("HF_HUB_DOWNLOAD_TIMEOUT", "21"),
        ("HF_HUB_DISABLE_IMPLICIT_TOKEN", "true"),
        ("HF_TOKEN", "  ab\r\ncd  "),
        ("HF_HUB_ENABLE_HF_TRANSFER", "yes"),
    ])
    .unwrap();
    assert_eq!(configured.endpoint, "https://hub.example");
    assert_eq!(
        configured.hf_home,
        PathBuf::from("/home/test/hf-cache/${MISSING}")
    );
    assert_eq!(configured.hub_cache, PathBuf::from("cache/hub"));
    assert_eq!(
        configured.token_path,
        PathBuf::from("/home/test/keys/token")
    );
    assert_eq!(
        configured.stored_tokens_path,
        PathBuf::from("/home/test/keys/stored_tokens")
    );
    assert_eq!(configured.etag_timeout, 0);
    assert_eq!(configured.download_timeout, 21);
    assert!(configured.token.disable_implicit);
    assert_eq!(
        configured.token.env_token.as_ref().unwrap().expose_secret(),
        "abcd"
    );
    assert_eq!(configured.transfer, TransferSetting::DeprecatedTransfer);
}

fn staging_follows_hub() {
    let default_staging = settings(&[("HUGGINGFACE_CO_STAGING", "true")]).unwrap();
    assert_eq!(default_staging.endpoint, "https://hub-ci.huggingface.co");
    let staging = settings(&[
        ("HUGGINGFACE_CO_STAGING", "true"),
        ("HF_ENDPOINT", "https://custom"),
        ("HOME", "/users/a"),
        ("HF_HOME", "/normal"),
        ("HF_HUB_CACHE", "/hub"),
    ])
    .unwrap();
    assert_eq!(staging.endpoint, "https://hub-ci.huggingface.co");
    assert_eq!(staging.hub_cache, PathBuf::from("/hub"));
    assert_eq!(
        staging.token_path,
        PathBuf::from("/users/a/.cache/huggingface_staging/token")
    );
    assert_eq!(
        staging.stored_tokens_path,
        PathBuf::from("/normal/stored_tokens")
    );
}

fn fallback_values_follow_hub() {
    assert_eq!(
        settings(&[("TRANSFORMERS_OFFLINE", "1")]).unwrap().offline,
        OfflineSetting::Offline
    );
    assert_eq!(
        settings(&[("HF_HUB_OFFLINE", "0"), ("TRANSFORMERS_OFFLINE", "1")])
            .unwrap()
            .offline,
        OfflineSetting::Online
    );
    assert_eq!(
        settings(&[("HF_TOKEN", ""), ("HUGGING_FACE_HUB_TOKEN", "fallback")])
            .unwrap()
            .token
            .env_token
            .unwrap()
            .expose_secret(),
        "fallback"
    );
    assert_eq!(
        settings(&[
            ("HF_XET_HIGH_PERFORMANCE", "true"),
            ("HF_HUB_ENABLE_HF_TRANSFER", "true")
        ])
        .unwrap()
        .transfer,
        TransferSetting::Standard
    );
    assert_eq!(
        settings(&[("XDG_CACHE_HOME", "/xdg")]).unwrap().hf_home,
        PathBuf::from("/xdg/huggingface")
    );
    assert_eq!(
        settings(&[("HUGGINGFACE_HUB_CACHE", "/legacy")])
            .unwrap()
            .hub_cache,
        PathBuf::from("/legacy")
    );
}

fn invalid_values_name_variables() {
    let invalid_flag = settings(&[("HF_HUB_DISABLE_IMPLICIT_TOKEN", "sometimes")])
        .unwrap_err()
        .to_string();
    assert!(invalid_flag.contains("HF_HUB_DISABLE_IMPLICIT_TOKEN"));
    let invalid_number = settings(&[("HF_HUB_ETAG_TIMEOUT", "NaN")])
        .unwrap_err()
        .to_string();
    assert!(invalid_number.contains("HF_HUB_ETAG_TIMEOUT"));
}
