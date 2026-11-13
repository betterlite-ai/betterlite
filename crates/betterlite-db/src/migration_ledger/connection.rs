use std::time::Duration;

use deadpool_postgres::{BuildError, Client, ClientWrapper, Manager, Pool, PoolError};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::database_tls::options::{ConnectionOptionsError, native_connection_options};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ConnectionError {
    #[error("the migration database URL could not be turned into connection options")]
    Options {
        #[source]
        source: ConnectionOptionsError,
    },
    #[error("the migration connection pool could not be built")]
    Pool {
        #[source]
        source: BuildError,
    },
    #[error("could not open the migration database connection")]
    Open {
        #[source]
        source: PoolError,
    },
}

// A migration session is single-use, so one detached connection with the ten-second connect timeout is the whole pool
pub async fn connect(database_url: &SecretString) -> Result<ClientWrapper, ConnectionError> {
    let options = native_connection_options(database_url.expose_secret())
        .map_err(|source| ConnectionError::Options { source })?;
    let mut config = options.config;
    config.connect_timeout(CONNECT_TIMEOUT);
    let pool = Pool::builder(Manager::new(config, options.tls))
        .max_size(1)
        .build()
        .map_err(|source| ConnectionError::Pool { source })?;
    let object = pool
        .get()
        .await
        .map_err(|source| ConnectionError::Open { source })?;
    Ok(Client::take(object))
}
