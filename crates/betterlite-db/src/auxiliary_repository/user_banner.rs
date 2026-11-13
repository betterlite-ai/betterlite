use std::future::Future;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions;
use deadpool_postgres::GenericClient;
use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;

use super::bindings::UISettings;
use super::repository::{PrismaClient, PrismaTableRepository, TableAccess, TableError};

pub const USER_BANNER_ROW_ID: &str = "user_banner";

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_UISettings\" (id, ui_settings, updated_at) \
     VALUES ($1, to_jsonb($2::text), now()) RETURNING to_jsonb(\"LiteLLM_UISettings\".*)::text";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_UISettings\" SET ui_settings = to_jsonb($2::text), \
     updated_at = now() WHERE id = $1 RETURNING to_jsonb(\"LiteLLM_UISettings\".*)::text";

const UPDATE_MANY_SQL: &str = "UPDATE \"LiteLLM_UISettings\" SET ui_settings = to_jsonb($2::text), \
     updated_at = now() WHERE id = $1";

const UPSERT_SQL: &str = "INSERT INTO \"LiteLLM_UISettings\" (id, ui_settings, updated_at) \
     VALUES ($1, to_jsonb($2::text), now()) ON CONFLICT (id) \
     DO UPDATE SET ui_settings = EXCLUDED.ui_settings, updated_at = now() \
     RETURNING to_jsonb(\"LiteLLM_UISettings\".*)::text";

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_UISettings\" WHERE id = $1 RETURNING to_jsonb(\"LiteLLM_UISettings\".*)::text";

const DELETE_MANY_SQL: &str = "DELETE FROM \"LiteLLM_UISettings\" WHERE id = $1";

const SELECT_SQL: &str = "SELECT ui_settings::text FROM \"LiteLLM_UISettings\" WHERE id = $1";

pub struct UiSettingsRow {
    pub id: String,
    pub ui_settings: String,
}

pub struct BannerActions<'db> {
    pub access: TableAccess<'db>,
}

impl BannerActions<'_> {
    fn row_json(row: &Row) -> Result<JsonData, TableError> {
        let text: String = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        serde_json::from_str(&text).map_err(TableError::Row)
    }
}

impl TableActions for BannerActions<'_> {
    type Row = JsonData;
    type Filter = String;
    type CreateData = UiSettingsRow;
    type UpdateData = String;
    type Failure = TableError;

    fn create(
        &self,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let row = client
                .query_one(CREATE_SQL, &[&data.id, &data.ui_settings])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            Self::row_json(&row)
        }
    }

    fn create_many(
        &self,
        data: Vec<Self::CreateData>,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let mut created = 0_usize;
            for row in &data {
                client
                    .execute(CREATE_SQL, &[&row.id, &row.ui_settings])
                    .await
                    .map_err(|source| TableError::Database(backend(source)))?;
                created = created.saturating_add(1);
            }
            Ok(created)
        }
    }

    fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let row = client
                .query_opt(UPDATE_SQL, &[&filter, &data])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            row.as_ref().map(Self::row_json).transpose()
        }
    }

    fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let updated = client
                .execute(UPDATE_MANY_SQL, &[&filter, &data])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            usize::try_from(updated).map_err(|source| TableError::RowCount { source })
        }
    }

    fn upsert(
        &self,
        _filter: Self::Filter,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let row = client
                .query_one(UPSERT_SQL, &[&data.id, &data.ui_settings])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            Self::row_json(&row)
        }
    }

    fn delete(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let row = client
                .query_opt(DELETE_SQL, &[&filter])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            row.as_ref().map(Self::row_json).transpose()
        }
    }

    fn delete_many(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move {
            let client = self.access.write.session().await?;
            let deleted = client
                .execute(DELETE_MANY_SQL, &[&filter])
                .await
                .map_err(|source| TableError::Database(backend(source)))?;
            usize::try_from(deleted).map_err(|source| TableError::RowCount { source })
        }
    }
}

pub struct UserBannerRepository<'db> {
    base: PrismaTableRepository<'db, UISettings>,
}

impl<'db> UserBannerRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, UISettings),
        }
    }

    pub async fn get_raw_settings(&self) -> Result<Option<JsonData>, TableError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let row = client
            .query_opt(SELECT_SQL, &[&USER_BANNER_ROW_ID])
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

    pub async fn upsert_settings(&self, payload: &str) -> Result<(), TableError> {
        let table = self.base.table(|access| BannerActions { access })?;
        let row = UiSettingsRow {
            id: USER_BANNER_ROW_ID.to_owned(),
            ui_settings: payload.to_owned(),
        };
        drop(table.upsert(USER_BANNER_ROW_ID.to_owned(), row).await?);
        Ok(())
    }
}
