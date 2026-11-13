use std::time::Duration;

use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout;

use crate::database_tls::options::{
    ConnectionOptionsError, NativeConnectionOptions, native_connection_options,
};

use super::RequestLogIndexError;
use super::database::PostgresDatabase;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STATEMENT_TIMEOUT_OFF: &str = "SET statement_timeout = 0";

pub(crate) fn connection_options(
    database_url: &str,
) -> Result<NativeConnectionOptions, RequestLogIndexError> {
    let mut options =
        native_connection_options(database_url).map_err(|source: ConnectionOptionsError| {
            RequestLogIndexError::ConnectionOptions { source }
        })?;
    options.config.connect_timeout(CONNECT_TIMEOUT);
    Ok(options)
}

pub(crate) async fn connect(
    database_url: &SecretString,
    budget: Duration,
) -> Result<PostgresDatabase, RequestLogIndexError> {
    match timeout(budget, open(database_url)).await {
        Err(_elapsed) => Err(RequestLogIndexError::ConnectTimeout),
        Ok(result) => result,
    }
}

pub(crate) async fn open(
    database_url: &SecretString,
) -> Result<PostgresDatabase, RequestLogIndexError> {
    let options = connection_options(database_url.expose_secret())?;
    let pool = Pool::builder(Manager::new(options.config, options.tls))
        .max_size(1)
        .build()
        .map_err(|source| RequestLogIndexError::Pool { source })?;
    let client = pool
        .get()
        .await
        .map_err(|source| RequestLogIndexError::Open { source })?;
    client
        .batch_execute(STATEMENT_TIMEOUT_OFF)
        .await
        .map_err(|source| RequestLogIndexError::Statement { source })?;
    Ok(PostgresDatabase { client })
}
