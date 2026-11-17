use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

pub type Headers = BTreeMap<String, String>;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    cohere_key: Option<SecretString>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    #[serde(default)]
    headers: Headers,
}

// The globals main.py's Cohere arms read: litellm.cohere_key, litellm.api_key, litellm.api_base and litellm.headers
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub cohere_api_key: Option<SecretString>,
    pub co_api_key: Option<SecretString>,
    pub cohere_api_base: Option<String>,
    pub cohere_key: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub headers: Headers,
}

impl Settings {
    // the chain's last operand is returned as-is, even empty; the earlier ones fall through
    #[must_use]
    pub fn resolve_api_key(&self, supplied: Option<&SecretString>) -> Option<SecretString> {
        let chain = [
            supplied,
            self.cohere_key.as_ref(),
            self.cohere_api_key.as_ref(),
            self.co_api_key.as_ref(),
        ];
        for candidate in chain {
            if let Some(key) = candidate.filter(|key| !key.expose_secret().is_empty()) {
                return Some(key.clone());
            }
        }
        self.api_key.clone()
    }

    // `api_base or litellm.api_base or COHERE_API_BASE`, with the route's default applied only after
    #[must_use]
    pub fn resolve_api_base(&self, supplied: Option<&str>) -> Option<String> {
        let base = supplied
            .filter(|text| !text.is_empty())
            .or_else(|| self.api_base.as_deref().filter(|text| !text.is_empty()))
            .or_else(|| {
                self.cohere_api_base
                    .as_deref()
                    .filter(|text| !text.is_empty())
            });
        base.map(str::to_owned)
    }

    // `headers = headers or litellm.headers or {}` followed by `headers.update(extra_headers)`
    #[must_use]
    pub fn resolve_headers(&self, supplied: Option<&Headers>, extra: &Headers) -> Headers {
        let mut headers = supplied
            .filter(|headers| !headers.is_empty())
            .map_or_else(|| self.headers.clone(), Clone::clone);
        headers.extend(extra.clone());
        headers
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut headers = Headers::new();
        for (name, value) in self.litellm_settings.headers {
            let resolved = resolve_string(value, "litellm_settings.headers", lookup)?;
            headers.insert(name, resolved);
        }
        Ok(Settings {
            cohere_api_key: configured(lookup.lookup("COHERE_API_KEY")),
            co_api_key: configured(lookup.lookup("CO_API_KEY")),
            cohere_api_base: lookup
                .lookup("COHERE_API_BASE")
                .filter(|text| !text.is_empty()),
            cohere_key: resolve_secret(
                self.litellm_settings.cohere_key,
                "litellm_settings.cohere_key",
                lookup,
            )?,
            api_key: resolve_secret(
                self.litellm_settings.api_key,
                "litellm_settings.api_key",
                lookup,
            )?,
            api_base: match self.litellm_settings.api_base {
                Some(base) => Some(resolve_string(base, "litellm_settings.api_base", lookup)?),
                None => None,
            },
            headers,
        })
    }
}

fn configured(value: Option<String>) -> Option<SecretString> {
    value
        .filter(|text| !text.is_empty())
        .map(SecretString::from)
}
