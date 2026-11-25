pub mod ports;
pub mod websearch_params;

mod overrides;
mod settings_value;
mod sso;

mod tests;

use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::reconnect::DbReconnect;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;
use thiserror::Error;

use self::overrides::apply_override;
use self::ports::{
    NoReconnect, SecretManagerSelection, SemanticFilterReload, SettingsRows, WebsearchInterception,
};
use self::settings_value::{
    enabled_flag, litellm_settings_object, semantic_config, semantic_filter_config,
};
use self::sso::apply_sso;
use self::websearch_params::{WebsearchParamError, handler_params};
use crate::config_override_support::decode::StoredOverrideError;
use crate::config_override_support::environment::RuntimeEnvironment;
use crate::config_override_support::models::OverrideKind;
use crate::config_override_support::state::{OverrideRuntimeState, SecretManagerInitError};
use crate::db_row_decrypt::warnings::Warnings;

const WEBSEARCH_KEY: &str = "websearch_interception_params";
const SEMANTIC_FILTER_KEY: &str = "mcp_semantic_tool_filter";

// One proxy's reload state: the last applied value of each row and everything the reloads act on.
pub struct SettingsReloader<Client: Send = NoReconnect> {
    pub cipher: DbCipher,
    pub environment: RuntimeEnvironment,
    pub control: Box<dyn SecretManagerSelection>,
    pub warnings: Warnings,
    pub state: OverrideRuntimeState,
    pub reconnect: Option<Client>,
    pub last_websearch: Option<JsonValue>,
    pub last_semantic_filter: Option<JsonValue>,
}

#[derive(Debug, Error)]
pub enum SettingsReloadError {
    #[error("could not read the settings row: {0}")]
    Database(#[from] DbError),
    #[error("a stored settings value is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("litellm_settings is not a JSON object")]
    LitellmSettingsNotAnObject,
    #[error(transparent)]
    WebsearchParams(#[from] WebsearchParamError),
    #[error("could not read the sso_settings row: {source}")]
    SsoRow {
        #[source]
        source: TableError,
        database: Box<DbError>,
    },
    #[error("sso_settings is not a JSON object")]
    SsoNotAnObject,
    #[error("mcp_semantic_tool_filter is not a JSON object")]
    SemanticFilterNotAnObject,
    #[error("mcp_semantic_tool_filter.enabled must be a boolean")]
    SemanticFilterEnabledNotBool,
    #[error("mcp_semantic_tool_filter.embedding_model must be a string")]
    EmbeddingModelNotString,
    #[error("mcp_semantic_tool_filter.top_k must be an integer")]
    SemanticFilterTopKNotInteger,
    #[error("mcp_semantic_tool_filter.similarity_threshold must be a number")]
    SimilarityThresholdNotNumber,
    #[error(transparent)]
    Override(#[from] StoredOverrideError),
    #[error(transparent)]
    SecretManager(#[from] SecretManagerInitError),
}

impl<Client: DbReconnect + Send> SettingsReloader<Client> {
    pub async fn reload_websearch<Db>(&mut self, db: &Db, callbacks: &dyn WebsearchInterception)
    where
        Db: SettingsRows + ?Sized,
    {
        if let Err(error) = self.apply_websearch(db, callbacks).await {
            tracing::error!("Error initializing web search interception settings from DB: {error}");
        }
    }

    pub async fn reload_semantic_filter<Db, Router>(
        &mut self,
        db: &Db,
        filter: Option<&dyn SemanticFilterReload<Router>>,
        router: Option<&Router>,
    ) where
        Db: SettingsRows + ?Sized,
        Router: ?Sized + Sync,
    {
        if let Err(error) = self.apply_semantic_filter(db, filter, router).await {
            tracing::error!("Error initializing semantic filter settings from DB: {error}");
        }
    }

    pub async fn reload_sso(&mut self, db: &dyn SsoSettingsRows) {
        if let Err(error) = apply_sso(self, db).await {
            tracing::error!(
                "litellm.proxy.proxy_server.py::ProxyConfig:_init_sso_settings_in_db - {error}"
            );
        }
    }

    pub async fn reload_vault(&mut self, db: &dyn SettingsRows) {
        let Err(error) = apply_override(self, db, OverrideKind::HashicorpVault).await else {
            return;
        };
        tracing::error!("Error loading Hashicorp Vault config override from DB: {error}");
    }

    pub async fn reload_cyberark(&mut self, db: &dyn SettingsRows) {
        let Err(error) = apply_override(self, db, OverrideKind::CyberArk).await else {
            return;
        };
        tracing::error!("Error loading CyberArk config override from DB: {error}");
    }

    async fn apply_websearch<Db>(
        &mut self,
        db: &Db,
        callbacks: &dyn WebsearchInterception,
    ) -> Result<(), SettingsReloadError>
    where
        Db: SettingsRows + ?Sized,
    {
        let Some(settings) = litellm_settings_object(db).await? else {
            return Ok(());
        };
        let Some(stored) = settings.get(WEBSEARCH_KEY) else {
            return Ok(());
        };
        let JsonValue::Object(config) = stored else {
            return Ok(());
        };
        if !config.contains_key("enabled") && self.last_websearch.is_none() {
            tracing::debug!(
                "Web search interception: stored settings carry no 'enabled' flag and none were applied before, so litellm_settings.callbacks keeps ownership of the callback."
            );
            return Ok(());
        }
        let enabled = enabled_flag(config);
        if matches!(
            self.last_websearch.as_ref(),
            Some(JsonValue::Object(previous)) if previous == config
        ) && callbacks.is_registered() == enabled
        {
            return Ok(());
        }
        reconfigure_websearch(callbacks, config, enabled)?;
        self.last_websearch = Some(stored.clone());
        Ok(())
    }

    async fn apply_semantic_filter<Db, Router>(
        &mut self,
        db: &Db,
        filter: Option<&dyn SemanticFilterReload<Router>>,
        router: Option<&Router>,
    ) -> Result<(), SettingsReloadError>
    where
        Db: SettingsRows + ?Sized,
        Router: ?Sized + Sync,
    {
        let Some(settings) = litellm_settings_object(db).await? else {
            return Ok(());
        };
        let Some(stored) = settings.get(SEMANTIC_FILTER_KEY) else {
            return Ok(());
        };
        let Some(stored_config) = semantic_config(stored)? else {
            return Ok(());
        };
        let Some(hook) = filter else {
            tracing::error!("no semantic filter is installed for 'mcp_semantic_tool_filter'");
            return Ok(());
        };
        if self.skip_semantic_rebuild(hook, stored_config) {
            return Ok(());
        }
        let config = semantic_filter_config(stored_config)?;
        let registered = hook.rebuild(&config, router).await;
        if config.enabled && registered {
            tracing::info!("MCP Semantic Filter reinitialized from DB");
        }
        if !config.enabled {
            tracing::info!("MCP Semantic Filter disabled");
        }
        self.last_semantic_filter = Some(stored.clone());
        Ok(())
    }

    fn skip_semantic_rebuild<Router>(
        &self,
        filter: &dyn SemanticFilterReload<Router>,
        config: &JsonMap,
    ) -> bool
    where
        Router: ?Sized + Sync,
    {
        if !matches!(
            self.last_semantic_filter.as_ref(),
            Some(JsonValue::Object(previous)) if previous == config
        ) {
            return false;
        }
        if filter.is_ready() {
            tracing::debug!("Semantic filter settings unchanged, skipping reinitialization");
            return true;
        }
        tracing::info!(
            "Semantic filter settings unchanged, but hook is missing or uninitialized. Reinitializing."
        );
        false
    }
}

fn reconfigure_websearch(
    callbacks: &dyn WebsearchInterception,
    config: &JsonMap,
    enabled: bool,
) -> Result<(), SettingsReloadError> {
    if enabled {
        let params = handler_params(config)?;
        callbacks.apply(Some(&params));
        tracing::info!("Web search interception reinitialized from DB");
    } else {
        callbacks.apply(None);
        tracing::info!("Web search interception disabled");
    }
    Ok(())
}
