use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_core::api::vector_store_registry::repository::VectorStoreDatabase;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_core::api::vector_tool_selection::database::VectorStoreSelectionDatabase;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::HelperFailure;
use super::raised;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Consumed,
    Selection,
    Lookup,
    Fallback,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) operation: Operation,
    #[serde(default)]
    pub(crate) stores: Vec<Value>,
    #[serde(default)]
    pub(crate) tools: Option<Vec<JsonObject>>,
    #[serde(default)]
    pub(crate) non_default_params: JsonObject,
    #[serde(default)]
    pub(crate) ids: Vec<String>,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) database: Option<DatabaseInput>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DatabaseInput {
    #[serde(default)]
    pub(crate) rows: Vec<Value>,
    #[serde(default)]
    pub(crate) verified: Vec<Value>,
    #[serde(default)]
    pub(crate) verify_failed: bool,
    #[serde(default)]
    pub(crate) read_failed: bool,
}

pub(crate) struct ScriptedDatabase {
    rows: Vec<LiteLlmManagedVectorStore>,
    verified: Vec<LiteLlmManagedVectorStore>,
    verify_failed: bool,
    read_failed: bool,
    recorded: Mutex<Vec<Value>>,
}

impl ScriptedDatabase {
    pub(crate) fn new(input: &DatabaseInput) -> Result<Self, HelperFailure> {
        Ok(Self {
            rows: decode_all(&input.rows)?,
            verified: decode_all(&input.verified)?,
            verify_failed: input.verify_failed,
            read_failed: input.read_failed,
            recorded: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<Value> {
        match self.recorded.lock() {
            Ok(calls) => calls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn record(&self, call: Value) {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn read_failure() -> TableError {
        TableError::Publish {
            message: "database read failed".to_owned(),
        }
    }
}

impl VectorStoreDatabase for ScriptedDatabase {
    fn stores(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStore>, TableError>> + Send {
        self.record(json!({"table": "stores", "order": {"created_at": "desc"}}));
        let rows = if self.read_failed {
            Err(Self::read_failure())
        } else {
            Ok(self.rows.clone())
        };
        ready(rows)
    }

    fn indexes(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError>> + Send {
        ready(Ok(Vec::new()))
    }
}

impl VectorStoreSelectionDatabase for ScriptedDatabase {
    fn find_store(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send {
        self.record(json!({"table": "stores", "where": {"vector_store_id": vector_store_id}}));
        if self.verify_failed {
            return ready(Err(Self::read_failure()));
        }
        let found = self
            .verified
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned();
        ready(Ok(found))
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

pub(crate) fn registry_of(stores: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    VectorStoreRegistry {
        stores,
        clock: Arc::new(Utc::now),
        credentials: Arc::new(CredentialSnapshot::default()),
    }
}
