use std::future::Future;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::TableActions;
use deadpool_postgres::GenericClient;
use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::backend;

use super::bindings::UISettings;
use super::repository::{
    Params, PrismaClient, PrismaTableRepository, TableAccess, TableError, query_json,
};

pub const USER_BANNER_ROW_ID: &str = "user_banner";

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_UISettings\" (id, ui_settings, updated_at) \
     VALUES ($1, to_jsonb($2::text), now()) RETURNING ui_settings::text";

const UPDATE_SQL: &str = "UPDATE \"LiteLLM_UISettings\" SET ui_settings = to_jsonb($2::text), \
     updated_at = now() WHERE id = $1 RETURNING ui_settings::text";

const UPDATE_MANY_SQL: &str = "UPDATE \"LiteLLM_UISettings\" SET ui_settings = to_jsonb($2::text), \
     updated_at = now() WHERE id = $1";

const UPSERT_SQL: &str = "INSERT INTO \"LiteLLM_UISettings\" (id, ui_settings, updated_at) \
     VALUES ($1, to_jsonb($2::text), now()) ON CONFLICT (id) \
     DO UPDATE SET ui_settings = EXCLUDED.ui_settings, updated_at = now() \
     RETURNING ui_settings::text";

const DELETE_SQL: &str =
    "DELETE FROM \"LiteLLM_UISettings\" WHERE id = $1 RETURNING ui_settings::text";

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

    async fn row(&self, sql: &str, params: &Params<'_>) -> Result<Option<Row>, TableError> {
        let client = self.access.write.session().await?;
        client
            .query_opt(sql, params)
            .await
            .map_err(|source| TableError::Database(backend(source)))
    }

    async fn one(&self, sql: &str, params: &Params<'_>) -> Result<JsonData, TableError> {
        let row = self.row(sql, params).await?.ok_or(TableError::MissingRow)?;
        Self::row_json(&row)
    }

    async fn optional(
        &self,
        sql: &str,
        params: &Params<'_>,
    ) -> Result<Option<JsonData>, TableError> {
        let row = self.row(sql, params).await?;
        row.as_ref().map(Self::row_json).transpose()
    }

    async fn changed(&self, sql: &str, params: &Params<'_>) -> Result<usize, TableError> {
        let client = self.access.write.session().await?;
        let rows = client
            .execute(sql, params)
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        usize::try_from(rows).map_err(|source| TableError::RowCount { source })
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
        async move { self.one(CREATE_SQL, &[&data.id, &data.ui_settings]).await }
    }

    fn create_many(
        &self,
        data: Vec<Self::CreateData>,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move {
            let mut created = 0_usize;
            for row in &data {
                self.changed(CREATE_SQL, &[&row.id, &row.ui_settings])
                    .await?;
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
        async move { self.optional(UPDATE_SQL, &[&filter, &data]).await }
    }

    fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move { self.changed(UPDATE_MANY_SQL, &[&filter, &data]).await }
    }

    fn upsert(
        &self,
        _filter: Self::Filter,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        async move { self.one(UPSERT_SQL, &[&data.id, &data.ui_settings]).await }
    }

    fn delete(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        async move { self.optional(DELETE_SQL, &[&filter]).await }
    }

    fn delete_many(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        async move { self.changed(DELETE_MANY_SQL, &[&filter]).await }
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
        query_json(access.read, SELECT_SQL, &[&USER_BANNER_ROW_ID]).await
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
