pub mod budget;
pub mod finops;

use std::sync::Arc;

use axum::Router;
use betterlite_config::loader::startup::process_environment;
use betterlite_db::database_tls::options::native_connection_options;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::OidcTokenCache;
use deadpool_postgres::{Manager, Pool};
use secrecy::ExposeSecret as _;

use crate::cache_activity_routes;
use crate::cache_activity_routes::store::{CacheActivityStore, DatabaseCacheActivityStore};
use crate::jobs::ptu::attribution::NoOidc;
use crate::state::ServerState;

pub(crate) fn mount(router: Router, state: &Arc<ServerState>) -> Router {
    match cache_activity_store(state) {
        Some(store) => router.merge(cache_activity_routes::routes(Some(store))),
        None => router.merge(cache_activity_routes::routes(None)),
    }
}

// The proxy reads the URL the CLI wrote before Prisma starts: general_settings.database_url, else DATABASE_URL, else the DATABASE_* parts.
fn cache_activity_store(state: &Arc<ServerState>) -> Option<Arc<dyn CacheActivityStore>> {
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
    let store: Arc<dyn CacheActivityStore> = Arc::new(DatabaseCacheActivityStore {
        store: Arc::new(postgres),
    });
    Some(store)
}

fn postgres_pool(database_url: &str) -> Option<Pool> {
    let options = match native_connection_options(database_url) {
        Ok(options) => options,
        Err(error) => {
            tracing::error!(%error, "the database URL cannot open the cache activity pool");
            return None;
        }
    };
    let manager = Manager::new(options.config, options.tls);
    match Pool::builder(manager).build() {
        Ok(pool) => Some(pool),
        Err(error) => {
            tracing::error!(%error, "the cache activity pool cannot be built");
            None
        }
    }
}
