use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_core::api::vector_store_registry::store::VectorStoreCredentials;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::HelperFailure;
use super::raised;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ConfigRecords,
    Mutations,
    DatabaseLookup,
    DatabaseRows,
    IdsAndToolParams,
    NamedCredentials,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) stores: Vec<Value>,
    #[serde(default)]
    pub(crate) indexes: Vec<Value>,
    #[serde(default)]
    pub(crate) entries: Vec<Value>,
    #[serde(default)]
    pub(crate) steps: Vec<Step>,
    #[serde(default)]
    pub(crate) lookups: Vec<String>,
    #[serde(default)]
    pub(crate) database: Option<DatabaseInput>,
    #[serde(default)]
    pub(crate) non_default_params: JsonObject,
    #[serde(default)]
    pub(crate) tools: Option<Vec<JsonObject>>,
    #[serde(default)]
    pub(crate) tool: Option<JsonObject>,
    #[serde(default)]
    pub(crate) credentials: BTreeMap<String, JsonObject>,
    #[serde(default)]
    pub(crate) ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Add { store: Value },
    Update { id: String, store: Value },
    Delete { id: String },
    IsConfig { id: String },
    UpsertIndex { index: Value },
    LookupIndex { name: String },
    IsIndex { name: String },
    Defaults,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DatabaseInput {
    #[serde(default)]
    pub(crate) stores: Vec<Value>,
    #[serde(default)]
    pub(crate) indexes: Vec<Value>,
    #[serde(default)]
    pub(crate) failing: bool,
}

pub(crate) struct ScriptedDatabase {
    stores: Vec<LiteLlmManagedVectorStore>,
    indexes: Vec<LiteLlmManagedVectorStoreIndex>,
    failing: bool,
    calls: Mutex<Vec<Value>>,
}

impl ScriptedDatabase {
    pub(crate) fn new(input: &DatabaseInput) -> Result<Self, HelperFailure> {
        Ok(Self {
            stores: decode_all(&input.stores)?,
            indexes: decode_all(&input.indexes)?,
            failing: input.failing,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, table: &'static str) -> Result<(), TableError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(json!({"table": table, "order": {"created_at": "desc"}}));
        drop(calls);
        if self.failing {
            return Err(TableError::Publish {
                message: "database read failed".to_owned(),
            });
        }
        Ok(())
    }
}

impl VectorStoreDatabase for ScriptedDatabase {
    async fn stores(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        self.record("stores")?;
        Ok(self.stores.clone())
    }

    async fn indexes(&self) -> Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError> {
        self.record("indexes")?;
        Ok(self.indexes.clone())
    }
}

pub(crate) struct ScriptedCredentials {
    pub(crate) values: BTreeMap<String, JsonObject>,
}

impl VectorStoreCredentials for ScriptedCredentials {
    fn credential_values(&self, credential_name: &str) -> JsonObject {
        self.values
            .get(credential_name)
            .cloned()
            .unwrap_or_default()
    }
}

pub(crate) fn decode_all<Record>(values: &[Value]) -> Result<Vec<Record>, HelperFailure>
where
    Record: DeserializeOwned,
{
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        items.push(decode(value)?);
    }
    Ok(items)
}

pub(crate) fn decode<Record>(value: &Value) -> Result<Record, HelperFailure>
where
    Record: DeserializeOwned,
{
    let decoded = serde_json::from_value(value.clone())
        .map_err(|error| raised("InvalidInput", error.to_string()))?;
    Ok(decoded)
}
