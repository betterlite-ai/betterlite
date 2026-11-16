use std::future::Future;

use betterlite_db::auxiliary_repository::bindings::ManagedObjectTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient as _;
use deadpool_postgres::tokio_postgres::Row;
use deadpool_postgres::tokio_postgres::types::ToSql;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::CONTAINER_OBJECT_PURPOSE;

// the columns of a managed-object row the ownership functions read, as Prisma hands them back
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ContainerOwnerRow {
    #[serde(default)]
    pub unified_object_id: Option<String>,
    pub model_object_id: String,
    pub file_purpose: String,
    #[serde(default)]
    pub created_by: Option<String>,
}

// record_container_owner's create data
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerOwnerWrite {
    pub unified_object_id: String,
    pub model_object_id: String,
    pub file_object: Json,
    pub file_purpose: String,
    pub created_by: String,
    pub updated_by: String,
}

// record_container_owner's update data: a tracked row keeps its purpose and its creator
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerOwnerUpdate {
    pub model_object_id: String,
    pub unified_object_id: String,
    pub file_object: Json,
    pub updated_by: String,
}

// LiteLLM's ManagedObjectRepository table calls, one method per Prisma call the ownership code makes
pub trait ContainerObjectTable: Send + Sync {
    fn availability(&self) -> Result<(), TableError>;

    fn find_unique(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send;

    fn find_container(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send;

    fn find_owned(
        &self,
        owner_scopes: Vec<String>,
    ) -> impl Future<Output = Result<Vec<String>, TableError>> + Send;

    fn create(
        &self,
        write: ContainerOwnerWrite,
    ) -> impl Future<Output = Result<(), TableError>> + Send;

    fn update(
        &self,
        update: ContainerOwnerUpdate,
    ) -> impl Future<Output = Result<(), TableError>> + Send;
}

macro_rules! row_json {
    ($filter:literal) => {
        concat!(
            "SELECT jsonb_build_object(",
            "'unified_object_id', row.unified_object_id, ",
            "'model_object_id', row.model_object_id, ",
            "'file_purpose', row.file_purpose, ",
            "'created_by', row.created_by",
            ")::text FROM \"LiteLLM_ManagedObjectTable\" AS row WHERE ",
            $filter
        )
    };
}

const FIND_UNIQUE_SQL: &str = row_json!("row.model_object_id = $1 LIMIT 1");

const FIND_CONTAINER_SQL: &str =
    row_json!("row.model_object_id = $1 AND row.file_purpose = $2 LIMIT 1");

const FIND_OWNED_SQL: &str = "SELECT row.model_object_id FROM \"LiteLLM_ManagedObjectTable\" AS row \
     WHERE row.file_purpose = $1 AND row.created_by = ANY($2::text[])";

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_ManagedObjectTable\" \
     (id, unified_object_id, model_object_id, file_object, file_purpose, created_by, updated_by, \
      updated_at) \
     VALUES ($1, $2, $3, $4::text::jsonb, $5, $6, $7, now())";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ManagedObjectTable\" AS row \
     SET unified_object_id = $2, file_object = $3::text::jsonb, updated_by = $4, \
     updated_at = now() \
     WHERE row.model_object_id = $1";

type Params<'query> = [&'query (dyn ToSql + Sync)];

fn decode_row(row: &Row) -> Result<ContainerOwnerRow, TableError> {
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

async fn query_row(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Option<ContainerOwnerRow>, TableError> {
    let client = store.session().await?;
    let found = client
        .query_opt(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    found.as_ref().map(decode_row).transpose()
}

async fn query_owned(
    store: &PostgresStore,
    owner_scopes: &[String],
) -> Result<Vec<String>, TableError> {
    let client = store.session().await?;
    let rows = client
        .query(FIND_OWNED_SQL, &[&CONTAINER_OBJECT_PURPOSE, &owner_scopes])
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    rows.iter()
        .map(|row| {
            row.try_get(0)
                .map_err(|source| TableError::Database(backend(source)))
        })
        .collect()
}

pub struct PrismaContainerObjects<'db> {
    base: PrismaTableRepository<'db, ManagedObjectTable>,
}

impl<'db> PrismaContainerObjects<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, ManagedObjectTable),
        }
    }
}

impl ContainerObjectTable for PrismaContainerObjects<'_> {
    fn availability(&self) -> Result<(), TableError> {
        self.base
            .prisma_client()
            .map(drop)
            .map_err(TableError::Database)
    }

    fn find_unique(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send {
        async move {
            let access = self.base.access()?;
            query_row(access.read, FIND_UNIQUE_SQL, &[&model_object_id]).await
        }
    }

    fn find_container(
        &self,
        model_object_id: &str,
    ) -> impl Future<Output = Result<Option<ContainerOwnerRow>, TableError>> + Send {
        async move {
            let access = self.base.access()?;
            query_row(
                access.read,
                FIND_CONTAINER_SQL,
                &[&model_object_id, &CONTAINER_OBJECT_PURPOSE],
            )
            .await
        }
    }

    fn find_owned(
        &self,
        owner_scopes: Vec<String>,
    ) -> impl Future<Output = Result<Vec<String>, TableError>> + Send {
        async move {
            let access = self.base.access()?;
            query_owned(access.read, &owner_scopes).await
        }
    }

    fn create(
        &self,
        write: ContainerOwnerWrite,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        async move {
            let access = self.base.access()?;
            let id = Uuid::new_v4().to_string();
            let file_object = serde_json::to_string(&write.file_object).map_err(TableError::Row)?;
            let client = access.write.session().await?;
            client
                .execute(
                    CREATE_SQL,
                    &[
                        &id,
                        &write.unified_object_id,
                        &write.model_object_id,
                        &file_object,
                        &write.file_purpose,
                        &write.created_by,
                        &write.updated_by,
                    ],
                )
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            Ok(())
        }
    }

    fn update(
        &self,
        update: ContainerOwnerUpdate,
    ) -> impl Future<Output = Result<(), TableError>> + Send {
        async move {
            let access = self.base.access()?;
            let file_object =
                serde_json::to_string(&update.file_object).map_err(TableError::Row)?;
            let client = access.write.session().await?;
            let changed = client
                .execute(
                    UPDATE_SQL,
                    &[
                        &update.model_object_id,
                        &update.unified_object_id,
                        &file_object,
                        &update.updated_by,
                    ],
                )
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            if changed == 0 {
                return Err(TableError::MissingRow);
            }
            Ok(())
        }
    }
}
