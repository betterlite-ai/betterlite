mod access;
mod error;
mod flow;
mod handlers;
mod lifecycle;
mod store;
mod wire;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{post, put};
use betterlite_config::loader::startup::process_environment;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, DB_NOT_CONNECTED, require_database,
};
use betterlite_db::database_tls::options::native_connection_options;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::OidcTokenCache;
use deadpool_postgres::{Manager, Pool};
use secrecy::ExposeSecret as _;

use crate::jobs::ptu::attribution::NoOidc;
use crate::mcp::outbound_credentials::{SystemWallClock, WallClock};
use crate::state::ServerState;

use self::store::{ClaudePluginStore, PostgresPluginStore};

const PLUGINS_PATH: &str = "/claude-code/plugins";
const PLUGIN_PATH: &str = "/claude-code/plugins/{plugin_name}";
const PLUGIN_ENABLE_PATH: &str = "/claude-code/plugins/{plugin_name}/enable";
const PLUGIN_DISABLE_PATH: &str = "/claude-code/plugins/{plugin_name}/disable";

pub(crate) struct ClaudePluginWriteState {
    pub(crate) store: Option<Arc<dyn ClaudePluginStore>>,
    pub(crate) clock: Arc<dyn WallClock>,
    pub(crate) limits: ApiLimits,
}

impl ClaudePluginWriteState {
    // _get_prisma_client: every mutation needs the database, and its 500 carries the pinned detail
    fn connected_store(&self) -> Result<Arc<dyn ClaudePluginStore>, ClaudePluginPolicyError> {
        require_database(self.store.is_some())?;
        self.store
            .clone()
            .ok_or(ClaudePluginPolicyError::DatabaseNotConnected {
                message: DB_NOT_CONNECTED,
            })
    }
}

pub(crate) fn routes(state: &Arc<ServerState>) -> Router {
    mount(ClaudePluginWriteState {
        store: database_store(state),
        clock: Arc::new(SystemWallClock),
        limits: ApiLimits::default(),
    })
}

fn mount(state: ClaudePluginWriteState) -> Router {
    Router::new()
        .route(PLUGINS_PATH, post(handlers::create_plugin))
        .route(
            PLUGIN_PATH,
            put(handlers::update_plugin).delete(handlers::delete_plugin),
        )
        .route(PLUGIN_ENABLE_PATH, post(handlers::enable_plugin))
        .route(PLUGIN_DISABLE_PATH, post(handlers::disable_plugin))
        // B1087's ApiLimits owns the JSON limit; axum's 2 MB extractor default would answer first
        .layer(DefaultBodyLimit::disable())
        .with_state(Arc::new(state))
}

// The proxy reads the URL the CLI wrote before Prisma starts: general_settings.database_url, else DATABASE_URL, else the DATABASE_* parts.
fn database_store(state: &Arc<ServerState>) -> Option<Arc<dyn ClaudePluginStore>> {
    let secrets = SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    };
    let resolved = state
        .config
        .sections
        .database_urls
        .cli_writer_url(&process_environment(), &secrets);
    let url = match resolved {
        Ok(Some(url)) => url,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, "the configured database URL cannot be resolved");
            return None;
        }
    };
    let pool = postgres_pool(url.expose_secret())?;
    let postgres = PostgresStore { pool: Some(pool) };
    let store: Arc<dyn ClaudePluginStore> = Arc::new(PostgresPluginStore::new(Arc::new(postgres)));
    Some(store)
}

fn postgres_pool(database_url: &str) -> Option<Pool> {
    let options = match native_connection_options(database_url) {
        Ok(options) => options,
        Err(error) => {
            tracing::error!(%error, "the database URL cannot open the plugin pool");
            return None;
        }
    };
    let manager = Manager::new(options.config, options.tls);
    match Pool::builder(manager).build() {
        Ok(pool) => Some(pool),
        Err(error) => {
            tracing::error!(%error, "the plugin pool cannot be built");
            None
        }
    }
}
