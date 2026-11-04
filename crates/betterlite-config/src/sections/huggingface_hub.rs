use std::path::PathBuf;

use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod paths;
mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub endpoint: String,
    pub offline: OfflineSetting,
    pub etag_timeout: u64,
    pub download_timeout: u64,
    pub disable_implicit_token: bool,
    pub env_token: Option<SecretString>,
    pub hf_home: PathBuf,
    pub hub_cache: PathBuf,
    pub token_path: PathBuf,
    pub stored_tokens_path: PathBuf,
    pub transfer: TransferSetting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OfflineSetting {
    #[default]
    Online,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferSetting {
    #[default]
    Standard,
    DeprecatedTransfer,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: "https://huggingface.co".to_owned(),
            offline: OfflineSetting::Online,
            etag_timeout: 10,
            download_timeout: 10,
            disable_implicit_token: false,
            env_token: None,
            hf_home: PathBuf::new(),
            hub_cache: PathBuf::new(),
            token_path: PathBuf::new(),
            stored_tokens_path: PathBuf::new(),
            transfer: TransferSetting::Standard,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let environment = HubEnvironment { lookup };
        let endpoint = environment.endpoint()?;
        let offline = environment.offline()?;
        let (hf_home, hub_cache, token_path, stored_tokens_path) = paths::resolve(lookup)?;
        let (disable_implicit_token, env_token) = resolve_token_settings(lookup, &environment)?;
        let transfer = resolve_transfer_setting(lookup)?;
        let etag_timeout_value = lookup.lookup("HF_HUB_ETAG_TIMEOUT").unwrap_or_default();
        let download_timeout_value = lookup.lookup("HF_HUB_DOWNLOAD_TIMEOUT").unwrap_or_default();
        Ok(Settings {
            endpoint,
            offline,
            etag_timeout: betterlite_env::number::<u64>(
                "HF_HUB_ETAG_TIMEOUT",
                &etag_timeout_value,
            )?
            .unwrap_or(10),
            download_timeout: betterlite_env::number::<u64>(
                "HF_HUB_DOWNLOAD_TIMEOUT",
                &download_timeout_value,
            )?
            .unwrap_or(10),
            disable_implicit_token,
            env_token,
            hf_home,
            hub_cache,
            token_path,
            stored_tokens_path,
            transfer,
        })
    }
}

struct HubEnvironment<'environment> {
    lookup: &'environment dyn EnvironmentLookup,
}

impl HubEnvironment<'_> {
    fn preferred(&self, first: &str, second: &str) -> String {
        let primary = self.lookup.lookup(first).unwrap_or_default();
        if primary.is_empty() {
            self.lookup.lookup(second).unwrap_or_default()
        } else {
            primary
        }
    }
    fn endpoint(&self) -> Result<String, ConfigError> {
        let staging = betterlite_env::flag(
            "HUGGINGFACE_CO_STAGING",
            &self
                .lookup
                .lookup("HUGGINGFACE_CO_STAGING")
                .unwrap_or_default(),
        )?
        .unwrap_or(false);
        let mut endpoint = if staging {
            "https://hub-ci.huggingface.co".to_owned()
        } else {
            self.lookup.lookup("HF_ENDPOINT").unwrap_or_default()
        };
        if endpoint.is_empty() {
            "https://huggingface.co".clone_into(&mut endpoint);
        }
        let trimmed = endpoint.trim_end_matches('/').to_owned();
        Ok(trimmed)
    }

    fn offline(&self) -> Result<OfflineSetting, ConfigError> {
        let configured = self.lookup.lookup("HF_HUB_OFFLINE").unwrap_or_default();
        let enabled = if configured.is_empty() {
            betterlite_env::flag(
                "TRANSFORMERS_OFFLINE",
                &self
                    .lookup
                    .lookup("TRANSFORMERS_OFFLINE")
                    .unwrap_or_default(),
            )?
            .unwrap_or(false)
        } else {
            betterlite_env::flag("HF_HUB_OFFLINE", &configured)?.unwrap_or(false)
        };
        Ok(if enabled {
            OfflineSetting::Offline
        } else {
            OfflineSetting::Online
        })
    }
}

fn resolve_token_settings(
    lookup: &dyn EnvironmentLookup,
    environment: &HubEnvironment<'_>,
) -> Result<(bool, Option<SecretString>), ConfigError> {
    let raw_token = environment.preferred("HF_TOKEN", "HUGGING_FACE_HUB_TOKEN");
    let cleaned_token = raw_token.replace(['\r', '\n'], "").trim().to_owned();
    let env_token = (!cleaned_token.is_empty()).then(|| SecretString::from(cleaned_token));
    let value = lookup
        .lookup("HF_HUB_DISABLE_IMPLICIT_TOKEN")
        .unwrap_or_default();
    let disable_implicit =
        betterlite_env::flag("HF_HUB_DISABLE_IMPLICIT_TOKEN", &value)?.unwrap_or(false);
    Ok((disable_implicit, env_token))
}

fn resolve_transfer_setting(
    lookup: &dyn EnvironmentLookup,
) -> Result<TransferSetting, ConfigError> {
    let transfer = lookup
        .lookup("HF_HUB_ENABLE_HF_TRANSFER")
        .unwrap_or_default();
    let xet = lookup.lookup("HF_XET_HIGH_PERFORMANCE").unwrap_or_default();
    let enabled = betterlite_env::flag("HF_HUB_ENABLE_HF_TRANSFER", &transfer)?.unwrap_or(false);
    let xet_enabled = betterlite_env::flag("HF_XET_HIGH_PERFORMANCE", &xet)?.unwrap_or(false);
    Ok(if enabled && !xet_enabled {
        TransferSetting::DeprecatedTransfer
    } else {
        TransferSetting::Standard
    })
}
