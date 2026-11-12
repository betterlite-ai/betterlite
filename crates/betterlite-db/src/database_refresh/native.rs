use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use deadpool_postgres::{BuildError, Client, Manager, Pool, PoolError};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::timeout as time_limit;

use crate::database_tls::options::{ConnectionOptionsError, native_connection_options};

use super::handle::{PoolFuture, PoolHandle, PoolProvider, PoolTransport};

#[derive(Debug, thiserror::Error)]
pub enum NativePoolError {
    #[error("the database connection options are invalid")]
    Options {
        #[source]
        source: ConnectionOptionsError,
    },
    #[error("the database pool could not be built")]
    Build {
        #[source]
        source: BuildError,
    },
    #[error("the database connection could not be opened")]
    Open {
        #[source]
        source: PoolError,
    },
    #[error("the database connection timed out")]
    Timeout,
    #[error("the database query failed")]
    Query {
        #[source]
        source: tokio_postgres::Error,
    },
}

pub struct NativePool {
    pub(crate) pool: Pool,
    pub(crate) connected: AtomicBool,
}

impl NativePool {
    pub async fn session(&self) -> Result<Client, NativePoolError> {
        self.pool
            .get()
            .await
            .map_err(|source| NativePoolError::Open { source })
    }
}

impl PoolHandle for NativePool {
    type Error = NativePoolError;

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            match timeout {
                Some(limit) => {
                    let _client = time_limit(limit, self.pool.get())
                        .await
                        .map_err(|_elapsed| NativePoolError::Timeout)?
                        .map_err(|source| NativePoolError::Open { source })?;
                }
                None => {
                    let _client = self
                        .pool
                        .get()
                        .await
                        .map_err(|source| NativePoolError::Open { source })?;
                }
            }
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn close(&self) {
        self.pool.close();
        self.connected.store(false, Ordering::SeqCst);
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn query<'borrowed>(
        &'borrowed self,
        sql: &'borrowed str,
    ) -> PoolFuture<'borrowed, u64, Self::Error> {
        Box::pin(async move {
            let client = self
                .pool
                .get()
                .await
                .map_err(|source| NativePoolError::Open { source })?;
            client
                .execute(sql, &[])
                .await
                .map_err(|source| NativePoolError::Query { source })
        })
    }
}

pub struct NativePoolProvider;

impl PoolProvider for NativePoolProvider {
    type Handle = NativePool;
    type Error = NativePoolError;

    fn build(
        &self,
        url: &SecretString,
        transport: Option<&PoolTransport>,
    ) -> Result<NativePool, NativePoolError> {
        let options = native_connection_options(url.expose_secret())
            .map_err(|source| NativePoolError::Options { source })?;
        let manager = Manager::new(options.config, options.tls);
        let mut builder = Pool::builder(manager);
        if let Some(supplied) = transport
            && let Some(max_size) = supplied.max_size
        {
            builder = builder.max_size(max_size);
        }
        let pool = builder
            .build()
            .map_err(|source| NativePoolError::Build { source })?;
        Ok(NativePool {
            pool,
            connected: AtomicBool::new(false),
        })
    }
}
