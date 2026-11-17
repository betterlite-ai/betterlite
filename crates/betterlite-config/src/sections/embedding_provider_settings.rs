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
    // `api_key or litellm.cohere_key or COHERE_API_KEY or CO_API_KEY or litellm.api_key`: every
    // operand but the last falls through when falsey, and the last is returned even when falsey
    #[must_use]
    pub fn resolve_api_key(&self, explicit: Option<&SecretString>) -> Option<SecretString> {
        let present =
            |key: Option<&SecretString>| key.filter(|key| !key.expose_secret().is_empty());
        present(explicit)
            .or_else(|| present(self.cohere_key.as_ref()))
            .or_else(|| present(self.cohere_api_key.as_ref()))
            .or_else(|| present(self.co_api_key.as_ref()))
            .or(self.api_key.as_ref())
            .cloned()
    }

    // `api_base or litellm.api_base or COHERE_API_BASE`, with the route's default applied only after
    #[must_use]
    pub fn resolve_api_base(&self, explicit: Option<&str>) -> Option<String> {
        let present = |value: Option<&'_ str>| value.filter(|text| !text.is_empty());
        present(explicit)
            .or_else(|| present(self.api_base.as_deref()))
            .or_else(|| present(self.cohere_api_base.as_deref()))
            .map(str::to_owned)
    }

    // `headers = headers or litellm.headers or {}` followed by `headers.update(extra_headers)`
    #[must_use]
    pub fn resolve_headers(&self, supplied: Option<&Headers>, extra: &Headers) -> Headers {
        let mut headers = match supplied.filter(|headers| !headers.is_empty()) {
            Some(headers) => headers.clone(),
            None => self.headers.clone(),
        };
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
    match value {
        Some(text) if !text.is_empty() => Some(SecretString::from(text)),
        Some(_) | None => None,
    }
}
