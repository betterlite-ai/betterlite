use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const CLARIFAI_BASE: &str = "https://api.clarifai.com/v2/ext/openai/v1";
const COMPACTIFAI_BASE: &str = "https://api.compactif.ai/v1";
const DATAROBOT_BASE: &str = "https://app.datarobot.com";
const FAKE_KEY: &str = "fake-api-key";

type Base<'base> = Option<&'base str>;
type Key<'key> = Option<&'key SecretString>;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub clarifai_api_key: Option<SecretString>,
    pub compactifai_api_key: Option<SecretString>,
    pub datarobot_api_token: Option<SecretString>,
    pub datarobot_endpoint: Option<String>,
}

impl Settings {
    #[must_use]
    pub fn resolve_clarifai(&self, base: Base<'_>, key: Key<'_>) -> (String, SecretString) {
        (
            resolved_base(base, None, CLARIFAI_BASE),
            resolved_key(key, self.clarifai_api_key.as_ref(), ""),
        )
    }

    #[must_use]
    pub fn resolve_compactifai(&self, base: Base<'_>, key: Key<'_>) -> (String, SecretString) {
        (
            resolved_base(base, None, COMPACTIFAI_BASE),
            resolved_key(key, self.compactifai_api_key.as_ref(), ""),
        )
    }

    #[must_use]
    pub fn resolve_datarobot(&self, base: Base<'_>, key: Key<'_>) -> (String, SecretString) {
        (
            resolved_base(base, self.datarobot_endpoint.as_deref(), DATAROBOT_BASE),
            resolved_key(key, self.datarobot_api_token.as_ref(), FAKE_KEY),
        )
    }
}

// the pinned providers read the caller's base and key with `or`, so an empty one falls through
fn resolved_base(explicit: Base<'_>, fallback: Base<'_>, default: &str) -> String {
    explicit
        .filter(|base| !base.is_empty())
        .map(str::to_owned)
        .or_else(|| fallback.map(str::to_owned))
        .unwrap_or_else(|| default.to_owned())
}

fn resolved_key(explicit: Key<'_>, fallback: Key<'_>, default: &str) -> SecretString {
    explicit
        .filter(|key| !key.expose_secret().is_empty())
        .or(fallback)
        .map_or_else(
            || SecretString::from(default.to_owned()),
            SecretString::clone,
        )
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let key = |name: &str| {
            lookup
                .lookup(name)
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        };
        Ok(Settings {
            clarifai_api_key: key("CLARIFAI_API_KEY"),
            compactifai_api_key: key("COMPACTIFAI_API_KEY"),
            datarobot_api_token: key("DATAROBOT_API_TOKEN"),
            // the pinned resolver tests the endpoint with `is None`, so a set-but-empty one still reaches the path edit
            datarobot_endpoint: lookup.lookup("DATAROBOT_ENDPOINT"),
        })
    }
}
