use std::future::Future;

use betterlite_db::auxiliary_repository::bindings::SkillsTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::PublishChange;
use deadpool_postgres::GenericClient as _;
use deadpool_postgres::tokio_postgres::Row;
use deadpool_postgres::tokio_postgres::types::ToSql;

use super::SkillPage;

// LiteLLM's SkillRepository: the handler awaits these five calls and nothing else
pub trait SkillRepository: Send + Sync {
    fn availability(&self) -> Result<(), TableError>;

    fn create(
        &self,
        write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send;

    fn find_unique(
        &self,
        skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send;

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send;

    fn delete(&self, skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillWrite {
    pub skill_id: String,
    pub display_title: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub source: String,
    pub metadata: Option<String>,
    pub file_content: Option<String>,
    pub file_name: Option<String>,
    pub file_type: Option<String>,
    pub created_by: String,
    pub updated_by: String,
}

// Prisma hands BYTEA to Python as Base64 text, so one expression shapes every returned row
macro_rules! skill_row {
    ($before:literal, $after:literal) => {
        concat!(
            $before,
            "to_jsonb(row) || jsonb_build_object('file_content', encode(row.file_content, 'base64'))",
            $after
        )
    };
}

const FIND_ONE_SQL: &str = skill_row!(
    "SELECT (",
    ")::text FROM \"LiteLLM_SkillsTable\" AS row WHERE row.skill_id = $1"
);

const FIND_PAGE_SQL: &str = skill_row!(
    "SELECT (",
    ")::text FROM \"LiteLLM_SkillsTable\" AS row \
     WHERE ($1::text[] IS NULL OR row.created_by = ANY($1::text[])) \
     ORDER BY row.created_at DESC LIMIT $2 OFFSET $3"
);

const CREATE_SQL: &str = skill_row!(
    "INSERT INTO \"LiteLLM_SkillsTable\" AS row \
     (skill_id, display_title, description, instructions, source, file_content, file_name, \
      file_type, metadata, created_by, updated_by) \
     VALUES ($1, $2, $3, $4, $5, decode($6, 'base64'), $7, $8, \
      COALESCE($9::text::jsonb, '{}'::jsonb), $10, $11) RETURNING (",
    ")::text"
);

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_SkillsTable\" AS row WHERE row.skill_id = $1";

type Params<'query> = [&'query (dyn ToSql + Sync)];

fn decode_row(row: &Row) -> Result<SkillTable, TableError> {
    let text: String = row
        .try_get(0)
        .map_err(|source| TableError::Database(backend(source)))?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

async fn query_rows(
    store: &PostgresStore,
    sql: &str,
    params: &Params<'_>,
) -> Result<Vec<SkillTable>, TableError> {
    let client = store.session().await?;
    let rows = client
        .query(sql, params)
        .await
        .map_err(|source| TableError::Database(backend(source)))?;
    rows.iter().map(decode_row).collect()
}

pub struct PrismaSkillRepository<'db> {
    base: PrismaTableRepository<'db, SkillsTable>,
}

impl<'db> PrismaSkillRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, SkillsTable),
        }
    }
}

impl SkillRepository for PrismaSkillRepository<'_> {
    fn availability(&self) -> Result<(), TableError> {
        self.base
            .prisma_client()
            .map(drop)
            .map_err(TableError::Database)
    }

    async fn create(&self, write: SkillWrite) -> Result<SkillTable, TableError> {
        let access = self.base.access()?;
        let rows = query_rows(
            access.write,
            CREATE_SQL,
            &[
                &write.skill_id,
                &write.display_title,
                &write.description,
                &write.instructions,
                &write.source,
                &write.file_content,
                &write.file_name,
                &write.file_type,
                &write.metadata,
                &write.created_by,
                &write.updated_by,
            ],
        )
        .await?;
        rows.into_iter().next().ok_or(TableError::MissingRow)
    }

    async fn find_unique(&self, skill_id: &str) -> Result<Option<SkillTable>, TableError> {
        let access = self.base.access()?;
        let rows = query_rows(access.read, FIND_ONE_SQL, &[&skill_id]).await?;
        Ok(rows.into_iter().next())
    }

    async fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> Result<Vec<SkillTable>, TableError> {
        let access = self.base.access()?;
        query_rows(
            access.read,
            FIND_PAGE_SQL,
            &[&created_by, &page.limit, &page.offset],
        )
        .await
    }

    async fn delete(&self, skill_id: &str) -> Result<(), TableError> {
        let access = self.base.access()?;
        let client = access.write.session().await?;
        let deleted = client
            .execute(DELETE_SQL, &[&skill_id])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        if deleted == 0 {
            return Err(TableError::MissingRow);
        }
        Ok(())
    }
}
