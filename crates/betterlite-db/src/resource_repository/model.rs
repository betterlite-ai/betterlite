use betterlite_redis::config_sync::publish::PublishChange;
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableBinding, TableError,
};
use crate::budget_permission_rows::JsonData;
use crate::postgres_store::errors::DbError;
use crate::resource_rows::ResourceRowsError;
use crate::resource_rows::model::ProxyModelTable;
use crate::stored_cipher::DbCipher;

use super::table::{KeyFilter, ResourceActions, ResourceBinding, RowWrite};

pub struct ProxyModelBinding;

impl TableBinding for ProxyModelBinding {
    const NAME: &'static str = "litellm_proxymodeltable";
    const TABLE: &'static str = "LiteLLM_ProxyModelTable";
}

impl ResourceBinding for ProxyModelBinding {
    const KEY: &'static str = "model_id";
    const ID: Option<&'static str> = Some("model_id");
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Table(#[from] TableError),
    #[error(transparent)]
    Row(#[from] ResourceRowsError),
}

pub struct ModelRepository<'db> {
    base: PrismaTableRepository<'db, ProxyModelBinding>,
    cipher: DbCipher,
}

impl<'db> ModelRepository<'db> {
    // The caller's encryption key never selects the read key; B2303's cipher does.
    #[must_use]
    pub fn new(
        client: Option<PrismaClient<'db>>,
        publish: PublishChange<TableError>,
        _encryption_key: Option<SecretString>,
        cipher: DbCipher,
    ) -> Self {
        Self {
            base: PrismaTableRepository::new(client, publish, ProxyModelBinding),
            cipher,
        }
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's BaseRepository prisma_client property
    pub const fn prisma_client(&self) -> Result<PrismaClient<'db>, DbError> {
        self.base.prisma_client()
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's table property builds the same actions
    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, ProxyModelBinding>>, DbError> {
        self.base.table(ResourceActions::new)
    }

    pub async fn find_all_except(
        &self,
        model_id: &str,
    ) -> Result<Vec<ProxyModelTable>, ModelError> {
        let rows = self
            .table()?
            .as_ref()
            .find::<JsonData>(KeyFilter::NotEqual(model_id))
            .await?;
        rows.into_iter().map(|row| self.to_model(row)).collect()
    }

    fn to_model(&self, row: JsonData) -> Result<ProxyModelTable, ModelError> {
        let mut model = ProxyModelTable::try_from(row)?;
        for (name, value) in &mut model.fields.litellm_params.0 {
            let JsonData::String(text) = value else {
                continue;
            };
            let stored = StoredValue::<()>::Text(SecretString::from(text.clone()));
            let policy = DecryptPolicy {
                log: FailureLog::Debug,
                value: FailureValue::Original,
            };
            if let Some(StoredValue::Text(opened)) = self.cipher.decrypt_value(stored, name, policy)
            {
                opened.expose_secret().clone_into(text);
            }
        }
        Ok(model)
    }
}
