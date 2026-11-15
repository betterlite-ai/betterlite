use std::collections::BTreeMap;

use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::{ConfigSyncTableActions, TableActions as _};

use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::resource_rows::ResourceRowsError;
use crate::resource_rows::credential::CredentialItem;
use crate::resource_rows::json_value_is_set;

use super::table::{ResourceActions, ResourceBinding, RowWrite};

pub struct CredentialBinding;

impl TableBinding for CredentialBinding {
    const NAME: &'static str = "litellm_credentialstable";
    const TABLE: &'static str = "LiteLLM_CredentialsTable";
}

impl ResourceBinding for CredentialBinding {
    const KEY: &'static str = "credential_name";
    const ID: Option<&'static str> = Some("credential_id");
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Row(#[from] ResourceRowsError),
    #[error("stored credential row has no credential_name")]
    MissingName,
}

pub struct CredentialsRepository<'db> {
    base: PrismaTableRepository<'db, CredentialBinding>,
}

impl<'db> CredentialsRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>, publish: PublishChange<TableError>) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, CredentialBinding),
        }
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's BaseRepository prisma_client property
    pub const fn prisma_client(&self) -> Result<PrismaClient<'db>, DbError> {
        self.base.prisma_client()
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's table property builds the same actions
    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, CredentialBinding>>, DbError> {
        self.base.table(ResourceActions::new)
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's one-query public read
    pub async fn find_all(&self) -> Result<Vec<JsonData>, CredentialError> {
        Ok(self.table()?.as_ref().find_many::<JsonData>().await?)
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's one-query public write
    pub async fn create(&self, data: RowWrite) -> Result<JsonData, CredentialError> {
        Ok(self.table()?.create(data).await?)
    }

    pub async fn find_by_name(
        &self,
        credential_name: &str,
    ) -> Result<Option<CredentialItem>, CredentialError> {
        let Some(row) = self
            .table()?
            .as_ref()
            .find_unique::<JsonData>(credential_name)
            .await?
        else {
            return Ok(None);
        };
        credential_from_row(row).map(Some)
    }

    pub async fn update_by_name(
        &self,
        credential_name: &str,
        data: RowWrite,
    ) -> Result<Option<JsonData>, CredentialError> {
        let table = self.table()?;
        let filter = credential_name.to_owned();
        Ok(table.update(data, filter).await?)
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's one-query public write
    pub async fn delete_by_name(
        &self,
        credential_name: &str,
    ) -> Result<Option<JsonData>, CredentialError> {
        Ok(self.table()?.delete(credential_name.to_owned()).await?)
    }
}

pub(super) fn credential_from_row(row: JsonData) -> Result<CredentialItem, CredentialError> {
    let JsonData::Object(mut fields) = row else {
        return Err(CredentialError::Row(ResourceRowsError::NotAnObject));
    };
    let Some(name) = fields.remove("credential_name") else {
        return Err(CredentialError::MissingName);
    };
    let mut model = BTreeMap::new();
    model.insert("credential_name".to_owned(), name);
    model.insert(
        "credential_values".to_owned(),
        credential_map(fields.remove("credential_values")),
    );
    model.insert(
        "credential_info".to_owned(),
        credential_map(fields.remove("credential_info")),
    );
    CredentialItem::try_from(JsonData::Object(model)).map_err(CredentialError::Row)
}

fn credential_map(field: Option<JsonData>) -> JsonData {
    match field {
        Some(value) if json_value_is_set(&value) => value,
        _ => JsonData::Object(BTreeMap::new()),
    }
}
