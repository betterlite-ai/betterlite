use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::settings::{JsonValue, Section, SettingsStore};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::table_ops::rows::RowError;
use betterlite_server::db_row_decrypt::warnings::Warnings;
use betterlite_server::proxy_config_state::state::ProxyConfigState;

use crate::cleanup_schedule::changes::CleanupScheduleState;
use crate::config_boot::raw_secret_manager::RawSecretManager;

// LiteLLM's _settings_stores order, which is the order the rows are read and the sections resolved
pub const STORE_SECTIONS: [Section; 4] = [
    Section::GeneralSettings,
    Section::RouterSettings,
    Section::LitellmSettings,
    Section::EnvironmentVariables,
];

pub const PASS_THROUGH_ENDPOINTS: &str = "pass_through_endpoints";

// The module globals LiteLLM assigns when a stored row changes; the running proxy applies each one
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEffect {
    Alerting(Vec<JsonValue>),
    AlertingArgs(JsonValue),
    AlertTypes(JsonValue),
    AlertToWebhookUrl(JsonValue),
    LitellmSetting { key: &'static str, value: JsonValue },
    UserUrlAllowedHosts(JsonValue),
    UserUrlValidation(bool),
    ProviderUrlAllowedHosts(JsonValue),
}

pub trait RuntimeSink: Send + Sync {
    fn apply(&self, effect: RuntimeEffect);
}

// get_config_param: the stored param_value of one LiteLLM_Config row, None when the row is missing
#[async_trait]
pub trait ConfigRowReader: Send + Sync {
    async fn config_param(&self, name: &str) -> Result<Option<JsonValue>, DbError<RowError>>;
}

pub struct OverlayDatabase {
    pub rows: Arc<dyn ConfigRowReader>,
    pub cipher: Arc<DbCipher>,
    pub warnings: Arc<Warnings>,
}

// What LiteLLM's ProxyConfig keeps between reloads besides the stores: the secret manager, the startup pass-through list and the warn-once memory
pub struct OverlayState {
    pub config: ProxyConfigState,
    pub secrets: RawSecretManager,
    pub runtime: Arc<dyn RuntimeSink>,
    pub startup_pass_through: Vec<JsonValue>,
    pub warned_shadowed: BTreeSet<(&'static str, String)>,
    pub cleanup: CleanupScheduleState,
}

#[must_use]
pub const fn store_mut(
    config: &mut ProxyConfigState,
    section: Section,
) -> Option<&mut SettingsStore> {
    match section {
        Section::GeneralSettings => Some(&mut config.general_settings),
        Section::RouterSettings => Some(&mut config.router_settings),
        Section::LitellmSettings => Some(&mut config.litellm_settings),
        Section::EnvironmentVariables => Some(&mut config.environment_variables),
        Section::UiSettings => None,
    }
}

// _as_settings_mapping: anything but a mapping reads as an empty one
#[must_use]
pub fn settings_mapping(value: Option<&JsonValue>) -> BTreeMap<String, JsonValue> {
    value
        .and_then(JsonValue::as_object)
        .map_or_else(BTreeMap::new, |fields| {
            fields
                .iter()
                .map(|(key, entry)| (key.clone(), entry.clone()))
                .collect()
        })
}
