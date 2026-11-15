use std::marker::PhantomData;
use std::num::TryFromIntError;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions};
use deadpool_postgres::GenericClient;
use serde::de::DeserializeOwned;
use tokio_postgres::types::ToSql;

use crate::database_replica::availability::Availability;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

use super::decoder::ManagedBatchRecordError;

// ast-grep-ignore: no-long-names
use betterlite_redis::config_sync::table_actions::wrap_table_actions_for_config_sync as wrap_for_sync;

#[derive(Debug, thiserror::Error)]
pub enum TableError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Batch(#[from] ManagedBatchRecordError),
    #[error("stored row is not valid JSON")]
    Row(#[source] serde_json::Error),
    #[error("statement returned no row")]
    MissingRow,
    #[error("database row count did not fit in usize")]
    RowCount {
        #[source]
        source: TryFromIntError,
    },
    #[error("config sync publication failed: {message}")]
    Publish { message: String },
}

pub trait TableBinding {
    const NAME: &'static str;
    const TABLE: &'static str;
}

#[derive(Clone, Copy)]
pub struct PrismaClient<'db> {
    pub writer: &'db PostgresStore,
    pub reader: &'db PostgresStore,
    pub availability: &'db Availability,
}

#[derive(Clone, Copy)]
pub struct TableAccess<'db> {
    pub read: &'db PostgresStore,
    pub write: &'db PostgresStore,
}

pub struct PrismaTableRepository<'db, T> {
    client: Option<PrismaClient<'db>>,
    use_writer: bool,
    publish: PublishChange<TableError>,
    _binding: PhantomData<T>,
}

impl<'db, T: TableBinding> PrismaTableRepository<'db, T> {
    #[must_use]
    pub fn new(
        client: Option<PrismaClient<'db>>,
        publish: PublishChange<TableError>,
        _binding: T,
    ) -> Self {
        Self {
            client,
            use_writer: false,
            publish,
            _binding: PhantomData,
        }
    }

    #[must_use]
    pub fn with_writer(mut self) -> Self {
        self.use_writer = true;
        self
    }

    pub fn prisma_client(&self) -> Result<PrismaClient<'db>, DbError> {
        let Some(client) = self.client else {
            return Err(DbError::NoDbConnected);
        };
        Ok(client)
    }

    pub fn access(&self) -> Result<TableAccess<'db>, DbError> {
        let client = self.prisma_client()?;
        if self.use_writer {
            return Ok(TableAccess {
                read: client.writer,
                write: client.writer,
            });
        }
        let read = if client
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
        {
            client.writer
        } else {
            client.reader
        };
        Ok(TableAccess {
            read,
            write: client.writer,
        })
    }

    pub fn table<A, Build>(&self, build: Build) -> Result<ConfigSyncTableActions<A>, DbError>
    where
        A: TableActions<Failure = TableError>,
        Build: FnOnce(TableAccess<'db>) -> A,
    {
        let access = self.access()?;
        Ok(wrap_for_sync(
            build(access),
            T::NAME,
            Arc::clone(&self.publish),
        ))
    }
}

pub(super) type Params<'a> = [&'a (dyn ToSql + Sync)];

pub(super) async fn query_json<T>(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Option<T>, TableError>
where
    T: DeserializeOwned,
{
    let client = store.session().await?;
    let row = client
        .query_opt(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(TableError::Row)
}
