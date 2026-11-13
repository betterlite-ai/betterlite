use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::settings::JsonValue;

pub const URL_VARIABLE: &str = "CLICKHOUSE_URL";
pub const READER_URL_VARIABLE: &str = "CLICKHOUSE_READER_URL";
pub const DATABASE_VARIABLE: &str = "CLICKHOUSE_DATABASE";
pub const DEFAULT_DATABASE: &str = "litellm";
const CLICKHOUSE_STORE: &str = "clickhouse";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawGeneralSettings {
    tracing: Option<RawTracing>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTracing {
    store: JsonValue,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enabled: bool,
    pub clickhouse_url: Option<SecretString>,
    pub clickhouse_reader_url: Option<SecretString>,
    pub clickhouse_database: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let enabled = matches!(
            self.general_settings.tracing.map(|tracing| tracing.store),
            Some(JsonValue::String(store)) if store == CLICKHOUSE_STORE
        );
        let clickhouse_url = lookup.lookup(URL_VARIABLE).map(SecretString::from);
        // os.getenv falls back only when the variable is absent; an explicit empty value reaches the receiver
        let clickhouse_reader_url = lookup
            .lookup(READER_URL_VARIABLE)
            .map(SecretString::from)
            .or_else(|| clickhouse_url.clone());
        let clickhouse_database = lookup
            .lookup(DATABASE_VARIABLE)
            .unwrap_or_else(|| DEFAULT_DATABASE.to_owned());
        Ok(Settings {
            enabled,
            clickhouse_url,
            clickhouse_reader_url,
            clickhouse_database,
        })
    }
}

pub fn decode_agent_tracing(bytes: &[u8]) -> Result<Settings, ConfigError> {
    let raw: Raw =
        serde_saphyr::from_slice(bytes).map_err(|source| ConfigError::Yaml { source })?;
    Section::convert(raw, &BTreeMap::<String, String>::new())
}

#[path = "agent_tracing/tests.rs"]
mod tests;
