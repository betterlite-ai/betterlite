use std::path::PathBuf;

use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub endpoint: String,
    pub offline: bool,
    pub etag_timeout: u64,
    pub download_timeout: u64,
    pub disable_implicit_token: bool,
    pub env_token: Option<SecretString>,
    pub hf_home: PathBuf,
    pub hub_cache: PathBuf,
    pub token_path: PathBuf,
    pub stored_tokens_path: PathBuf,
    pub hf_transfer_deprecated: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: "https://huggingface.co".to_owned(),
            offline: false,
            etag_timeout: 10,
            download_timeout: 10,
            disable_implicit_token: false,
            env_token: None,
            hf_home: PathBuf::new(),
            hub_cache: PathBuf::new(),
            token_path: PathBuf::new(),
            stored_tokens_path: PathBuf::new(),
            hf_transfer_deprecated: false,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let value = |name: &str| lookup.lookup(name).unwrap_or_default();
        let expanded = |name: &str, fallback: String| {
            expand_path(&value(name), lookup).unwrap_or_else(|_| PathBuf::from(fallback))
        };
        let staging =
            betterlite_env::flag("HUGGINGFACE_CO_STAGING", &value("HUGGINGFACE_CO_STAGING"))?
                .unwrap_or(false);
        let mut endpoint = if staging {
            "https://hub-ci.huggingface.co".to_owned()
        } else {
            value("HF_ENDPOINT")
        };
        if endpoint.is_empty() {
            endpoint = "https://huggingface.co".to_owned();
        }
        endpoint = endpoint.trim_end_matches('/').to_owned();
        let offline_value = value("HF_HUB_OFFLINE");
        let offline = if offline_value.is_empty() {
            betterlite_env::flag("TRANSFORMERS_OFFLINE", &value("TRANSFORMERS_OFFLINE"))?
                .unwrap_or(false)
        } else {
            betterlite_env::flag("HF_HUB_OFFLINE", &offline_value)?.unwrap_or(false)
        };
        let hf_home = if !value("HF_HOME").is_empty() {
            expanded("HF_HOME", String::new())
        } else if !value("XDG_CACHE_HOME").is_empty() {
            PathBuf::from(expanded("XDG_CACHE_HOME", String::new())).join("huggingface")
        } else {
            PathBuf::from(expanded(
                "HOME",
                lookup
                    .system_home()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ))
            .join(".cache/huggingface")
        };
        let hub_cache_value = if !value("HF_HUB_CACHE").is_empty() {
            value("HF_HUB_CACHE")
        } else {
            value("HUGGINGFACE_HUB_CACHE")
        };
        let hub_cache = if hub_cache_value.is_empty() {
            hf_home.join("hub")
        } else {
            expand_path(&hub_cache_value, lookup)?
        };
        let token_path = if staging {
            PathBuf::from(expand_path("~/.cache/huggingface_staging/token", lookup)?)
        } else if value("HF_TOKEN_PATH").is_empty() {
            hf_home.join("token")
        } else {
            expanded("HF_TOKEN_PATH", String::new())
        };
        let stored_tokens_path = token_path
            .parent()
            .unwrap_or(&token_path)
            .join("stored_tokens");
        let env_token = if value("HF_TOKEN").is_empty() {
            value("HUGGING_FACE_HUB_TOKEN")
        } else {
            value("HF_TOKEN")
        };
        let env_token = {
            let cleaned = env_token.replace(['\r', '\n'], "").trim().to_owned();
            (!cleaned.is_empty()).then(|| SecretString::from(cleaned))
        };
        Ok(Settings {
            endpoint,
            offline,
            etag_timeout: betterlite_env::number::<u64>(
                "HF_HUB_ETAG_TIMEOUT",
                &value("HF_HUB_ETAG_TIMEOUT"),
            )?
            .unwrap_or(10),
            download_timeout: betterlite_env::number::<u64>(
                "HF_HUB_DOWNLOAD_TIMEOUT",
                &value("HF_HUB_DOWNLOAD_TIMEOUT"),
            )?
            .unwrap_or(10),
            disable_implicit_token: betterlite_env::flag(
                "HF_HUB_DISABLE_IMPLICIT_TOKEN",
                &value("HF_HUB_DISABLE_IMPLICIT_TOKEN"),
            )?
            .unwrap_or(false),
            env_token,
            hf_home,
            hub_cache,
            token_path,
            stored_tokens_path,
            hf_transfer_deprecated: betterlite_env::flag(
                "HF_HUB_ENABLE_HF_TRANSFER",
                &value("HF_HUB_ENABLE_HF_TRANSFER"),
            )?
            .unwrap_or(false)
                && !betterlite_env::flag(
                    "HF_XET_HIGH_PERFORMANCE",
                    &value("HF_XET_HIGH_PERFORMANCE"),
                )?
                .unwrap_or(false),
        })
    }
}

fn expand_path(value: &str, lookup: &dyn EnvironmentLookup) -> Result<PathBuf, ConfigError> {
    let home = lookup.lookup("HOME").or_else(|| {
        lookup
            .system_home()
            .map(|path| path.to_string_lossy().into_owned())
    });
    let expanded_home = match (value.strip_prefix("~/"), value == "~") {
        (Some(rest), _) => Some(format!(
            "{}{rest}",
            home.unwrap_or_default().trim_end_matches('/')
        )),
        (_, true) => home,
        _ => None,
    };
    let source = expanded_home.as_deref().unwrap_or(value);
    let mut output = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }
        let (name, braced) = if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                name.push(next);
            }
            (name, true)
        } else {
            let mut name = String::new();
            while let Some(next) = chars.peek() {
                if next.is_ascii_alphanumeric() || *next == '_' {
                    name.push(*next);
                    chars.next();
                } else {
                    break;
                }
            }
            (name, false)
        };
        if name.is_empty() {
            output.push('$');
            if braced {
                output.push('{');
                output.push('}');
            }
        } else if let Some(replacement) = lookup.lookup(&name) {
            output.push_str(&replacement);
        } else {
            output.push('$');
            if braced {
                output.push('{');
                output.push_str(&name);
                output.push('}');
            } else {
                output.push_str(&name);
            }
        }
    }
    Ok(PathBuf::from(output))
}
