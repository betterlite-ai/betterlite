pub mod budget;
pub mod finops;

use std::future::{Ready, ready};
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
    match cli_writer_store(state) {
        Some(store) => {
            let cache: Arc<dyn CacheActivityStore> = Arc::new(DatabaseCacheActivityStore { store });
            router.merge(cache_activity_routes::routes(Some(cache)))
        }
        None => router.merge(cache_activity_routes::routes(None)),
    }
}

#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn mount_public(router: Router, _state: &Arc<ServerState>) -> Router {
    // the area owner adds its public routes
    router.merge(Router::new())
}

#[cfg_attr(test, mutants::skip)]
pub fn start(_state: &Arc<ServerState>) -> Ready<()> {
    // the area owner fills this startup step
    ready(())
}

// The proxy reads the URL the CLI wrote before Prisma starts: general_settings.database_url, else DATABASE_URL, else the DATABASE_* parts.
pub(crate) fn cli_writer_store(state: &Arc<ServerState>) -> Option<Arc<PostgresStore>> {
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
    Some(Arc::new(PostgresStore { pool: Some(pool) }))
}

fn postgres_pool(database_url: &str) -> Option<Pool> {
    let options = match native_connection_options(database_url) {
        Ok(options) => options,
        Err(error) => {
            tracing::error!(%error, "the database URL cannot open the proxy pool");
            return None;
        }
    };
    let manager = Manager::new(options.config, options.tls);
    match Pool::builder(manager).build() {
        Ok(pool) => Some(pool),
        Err(error) => {
            tracing::error!(%error, "the proxy pool cannot be built");
            None
        }
    }
}
