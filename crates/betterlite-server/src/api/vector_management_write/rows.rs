use std::collections::BTreeMap;
use std::future::Future;

use betterlite_db::auxiliary_repository::bindings::ManagedVectorStoresTable as StoresBinding;
use betterlite_db::auxiliary_repository::repository::{
    PrismaTableRepository, TableBinding, TableError,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::resource_repository::table::{ResourceActions, ResourceBinding, RowWrite};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redis::config_sync::publish::{PublishChange, redis_config_change_publisher};
use betterlite_redis::config_sync::table_actions::TableActions;
use deadpool_postgres::GenericClient as _;

use crate::api::vector_management_boundaries::rows::VectorStoreRows;
use crate::api::vector_management_read::database::PostgresVectorDatabase;

// B858's vector store table, keyed and stamped the way B726's row actions need it
pub struct StoresTable;

impl TableBinding for StoresTable {
    const NAME: &'static str = <StoresBinding as TableBinding>::NAME;
    const TABLE: &'static str = <StoresBinding as TableBinding>::TABLE;
}

impl ResourceBinding for StoresTable {
    const KEY: &'static str = "vector_store_id";
    const ID: Option<&'static str> = None;
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

// data_to_create: the id, the provider and the params always, the optional fields only when the caller gave them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVectorStoreRecord {
    pub vector_store_id: String,
    pub custom_llm_provider: String,
    pub vector_store_name: Option<String>,
    pub vector_store_description: Option<String>,
    pub vector_store_metadata: Option<String>,
    pub litellm_credential_name: Option<String>,
    pub litellm_params: String,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
}

impl NewVectorStoreRecord {
    #[must_use]
    pub fn document(&self) -> BTreeMap<String, JsonData> {
        let mut document = BTreeMap::new();
        document.insert(
            "vector_store_id".to_owned(),
            JsonData::String(self.vector_store_id.clone()),
        );
        document.insert(
            "custom_llm_provider".to_owned(),
            JsonData::String(self.custom_llm_provider.clone()),
        );
        document.insert(
            "litellm_params".to_owned(),
            JsonData::String(self.litellm_params.clone()),
        );
        let optional: [(&str, &Option<String>); 5] = [
            ("vector_store_name", &self.vector_store_name),
            ("vector_store_description", &self.vector_store_description),
            ("vector_store_metadata", &self.vector_store_metadata),
            ("litellm_credential_name", &self.litellm_credential_name),
            ("team_id", &self.team_id),
        ];
        for (name, value) in optional {
            if let Some(text) = value {
                document.insert(name.to_owned(), JsonData::String(text.clone()));
            }
        }
        if let Some(user_id) = &self.user_id {
            document.insert("user_id".to_owned(), JsonData::String(user_id.clone()));
        }
        document
    }
}

// exclude_unset: an omitted field is never written, an explicit null is written as null, any other value as it came
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldChange<T> {
    Omitted,
    Null,
    Value(T),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorStoreChanges {
    pub custom_llm_provider: FieldChange<String>,
    pub vector_store_name: FieldChange<String>,
    pub vector_store_description: FieldChange<String>,
    pub vector_store_metadata: FieldChange<String>,
}

impl VectorStoreChanges {
    #[must_use]
    pub fn document(&self) -> BTreeMap<String, JsonData> {
        let mut document = BTreeMap::new();
        let changes: [(&str, &FieldChange<String>); 4] = [
            ("custom_llm_provider", &self.custom_llm_provider),
            ("vector_store_name", &self.vector_store_name),
            ("vector_store_description", &self.vector_store_description),
            ("vector_store_metadata", &self.vector_store_metadata),
        ];
        for (name, change) in changes {
            match change {
                FieldChange::Omitted => {}
                FieldChange::Null => {
                    document.insert(name.to_owned(), JsonData::Null);
                }
                FieldChange::Value(text) => {
                    document.insert(name.to_owned(), JsonData::String(text.clone()));
                }
            }
        }
        document
    }
}

// the three repository writes the management routes make, so a test answers them without Postgres
pub trait VectorStoreWrites: VectorStoreRows {
    fn create(
        &self,
        record: NewVectorStoreRecord,
    ) -> impl Future<Output = Result<LiteLlmManagedVectorStore, TableError>> + Send;

    fn update(
        &self,
        vector_store_id: &str,
        changes: VectorStoreChanges,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send;

    fn delete(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send;
}

// an insert whose updated_at comes from the database clock, as Prisma's @updatedAt would set it
fn insert_sql(document: &BTreeMap<String, JsonData>) -> String {
    let values: Vec<String> = document
        .keys()
        .map(|name| format!("r.\"{name}\""))
        .collect();
    let mut columns: Vec<String> = document.keys().map(|name| format!("\"{name}\"")).collect();
    columns.push(String::from("\"updated_at\""));
    format!(
        "INSERT INTO \"{}\" AS t ({}) SELECT {}, now() FROM jsonb_populate_record(NULL::\"{}\", $1::text::jsonb) AS r RETURNING row_to_json(t)::text",
        StoresTable::TABLE,
        columns.join(", "),
        values.join(", "),
        StoresTable::TABLE
    )
}

fn row_record(row: JsonData) -> Result<LiteLlmManagedVectorStore, TableError> {
    let text = serde_json::to_string(&row).map_err(TableError::Row)?;
    serde_json::from_str(&text).map_err(TableError::Row)
}

// B726's row actions over B858's table, reached through B1070's database handle
impl VectorStoreWrites for PostgresVectorDatabase {
    #[cfg_attr(test, mutants::skip)]
    async fn create(
        &self,
        record: NewVectorStoreRecord,
    ) -> Result<LiteLlmManagedVectorStore, TableError> {
        let document = record.document();
        let sql = insert_sql(&document);
        let json = serde_json::to_string(&document).map_err(TableError::Row)?;
        let client = self.client();
        let session = client
            .writer
            .session()
            .await
            .map_err(TableError::Database)?;
        let row = session
            .query_one(&sql, &[&json])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        let text: String = row
            .try_get(0)
            .map_err(|source| TableError::Database(backend(source)))?;
        let publish: PublishChange<TableError> = redis_config_change_publisher(None);
        publish(StoresTable::NAME).await?;
        serde_json::from_str(&text).map_err(TableError::Row)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn update(
        &self,
        vector_store_id: &str,
        changes: VectorStoreChanges,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        let repository = PrismaTableRepository::new(
            Some(self.client()),
            redis_config_change_publisher(None),
            StoresTable,
        );
        let table = repository
            .table(ResourceActions::<StoresTable>::new)
            .map_err(TableError::Database)?;
        let updated = TableActions::update(
            &table,
            RowWrite(changes.document()),
            vector_store_id.to_owned(),
        )
        .await?;
        updated.map(row_record).transpose()
    }

    #[cfg_attr(test, mutants::skip)]
    async fn delete(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        let repository = PrismaTableRepository::new(
            Some(self.client()),
            redis_config_change_publisher(None),
            StoresTable,
        );
        let table = repository
            .table(ResourceActions::<StoresTable>::new)
            .map_err(TableError::Database)?;
        let deleted = TableActions::delete(&table, vector_store_id.to_owned()).await?;
        deleted.map(row_record).transpose()
    }
}
