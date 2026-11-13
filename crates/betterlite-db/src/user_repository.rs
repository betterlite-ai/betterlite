pub mod email;
pub mod placeholders;
pub mod projection;
pub mod reporting;
pub mod tables;
mod tests;

use deadpool_postgres::GenericClient;

use crate::identity_rows::user::{LiteLLMUserTable, ScimPlaceholder};
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::{Key, KeyAction, KeyColumn};
use crate::resource_rows::ResourceRowsError;
use crate::resource_rows::organization::OrganizationTable;
use crate::resource_rows::project::ProjectTable;
use projection::UserRecordError;
use tables::{StoredTable, USER_TABLE};

#[derive(Debug, thiserror::Error)]
pub enum RowDecodeError {
    #[error("the row column is unreadable")]
    Column {
        #[source]
        source: DriverError,
    },
    #[error("the stored user record is not a readable row")]
    User {
        #[source]
        source: UserRecordError,
    },
    #[error("the stored row is not a readable record")]
    Row {
        #[source]
        source: ResourceRowsError,
    },
}

pub type StoredLookup<M> = Result<Option<M>, DbError<RowDecodeError>>;

pub struct UserRepository<'database, C> {
    pub db: Option<&'database C>,
    pub writer_db: Option<&'database C>,
    pub use_writer: bool,
}

pub struct StoredRepository<'database, C, M> {
    pub db: Option<&'database C>,
    pub table: StoredTable<M>,
}

pub type OrganizationRepository<'database, C> = StoredRepository<'database, C, OrganizationTable>;
pub type ProjectRepository<'database, C> = StoredRepository<'database, C, ProjectTable>;

impl<'database, C> UserRepository<'database, C> {
    // LiteLLM's writer_db property is db itself when no separate writer handle is configured
    fn client(&self) -> Option<&'database C> {
        if self.use_writer {
            self.writer_db.or(self.db)
        } else {
            self.db
        }
    }
}

impl<C> UserRepository<'_, C>
where
    C: GenericClient + Sync,
{
    pub async fn find_by_id(&self, id_value: &str) -> StoredLookup<LiteLLMUserTable> {
        let stored = StoredRepository {
            db: self.client(),
            table: USER_TABLE,
        };
        stored.find_by_id(id_value).await
    }

    pub async fn find_by_emails(
        &self,
        user_emails: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, DbError<RowDecodeError>> {
        let pages = email::email_pages(user_emails);
        if pages.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.client().ok_or(DbError::NoDbConnected)?;
        let mut users = Vec::new();
        for page in pages {
            let rows = client
                .query(email::EMAIL_PAGE_SQL, &[&page])
                .await
                .map_err(backend)?;
            let decoded = rows
                .iter()
                .map(tables::decode_user_row)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|source| DbError::InvalidRecord { source })?;
            users.extend(decoded.into_iter().flatten());
        }
        Ok(users)
    }

    pub async fn find_shadowing_placeholders<Session>(
        &self,
        session: &Session,
    ) -> Result<Vec<ScimPlaceholder>, DbError<placeholders::PlaceholderError>>
    where
        Session: GenericClient + Sync,
    {
        let rows = session
            .query(placeholders::SHADOWING_PLACEHOLDERS_SQL, &[])
            .await
            .map_err(backend)?;
        rows.iter()
            .map(placeholders::placeholder_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| DbError::InvalidRecord { source })
    }

    pub async fn count_billable_users(&self) -> Result<i64, DbError<RowDecodeError>> {
        let client = self.client().ok_or(DbError::NoDbConnected)?;
        let total: i64 = client
            .query_one(reporting::USER_COUNT_SQL, &[])
            .await
            .map_err(backend)?
            .try_get(0)
            .map_err(backend)?;
        let deactivated: i64 = client
            .query_one(reporting::USER_SCIM_DEACTIVATED_SQL, &[])
            .await
            .map_err(backend)?
            .try_get(0)
            .map_err(backend)?;
        Ok(total.saturating_sub(deactivated).max(0))
    }

    pub async fn backfill_null_user_email(
        &self,
        user_id: &str,
        user_email: &str,
    ) -> Result<u64, DbError<RowDecodeError>> {
        let client = self.client().ok_or(DbError::NoDbConnected)?;
        client
            .execute(reporting::BACKFILL_SQL, &[&user_id, &user_email])
            .await
            .map_err(backend)
    }
}

impl<C, M> StoredRepository<'_, C, M>
where
    C: GenericClient + Sync,
{
    pub async fn find_by_id(&self, id_value: &str) -> StoredLookup<M> {
        let key = Key {
            column: KeyColumn::Id,
            value: &id_value,
        };
        Repository {
            client: self.db,
            table: self.table,
        }
        .by_key(KeyAction::Find, key)
        .await
    }
}
