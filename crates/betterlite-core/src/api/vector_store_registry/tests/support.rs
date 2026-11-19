use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::{
    IndexCreateLiteLlmParams, LiteLlmManagedVectorStoreIndex,
};
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level};
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::api::vector_store_registry::repository::VectorStoreDatabase;
use crate::api::vector_store_registry::store::{
    RegistryClock, VectorStoreCredentials, VectorStoreRegistry,
};

pub(crate) const FIXED_TIMESTAMP: &str = "2026-01-02T03:04:05.000000Z";

pub(crate) fn fixed_clock() -> RegistryClock {
    let fixed = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .expect("the fixed instant parses")
        .with_timezone(&Utc);
    Arc::new(move || fixed)
}

fn decoded<Record, Document>(document: Document) -> Record
where
    Record: DeserializeOwned,
    Document: Serialize,
{
    let encoded = serde_json::to_value(document).expect("the store document encodes");
    serde_json::from_value(encoded).expect("the store document decodes")
}

pub(crate) fn registry(stores: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    VectorStoreRegistry::new(
        stores,
        fixed_clock(),
        Arc::new(CredentialSnapshot::default()),
    )
}

pub(crate) fn registry_with_credentials(
    stores: Vec<LiteLlmManagedVectorStore>,
    credentials: Arc<dyn VectorStoreCredentials>,
) -> VectorStoreRegistry {
    VectorStoreRegistry::new(stores, fixed_clock(), credentials)
}

pub(crate) fn store(id: &str) -> LiteLlmManagedVectorStore {
    decoded(serde_json::json!({
        "vector_store_id": id,
        "custom_llm_provider": "openai",
        "vector_store_description": "listed",
    }))
}

pub(crate) fn config_store(id: &str) -> LiteLlmManagedVectorStore {
    decoded(serde_json::json!({
        "vector_store_id": id,
        "custom_llm_provider": "openai",
        "is_config": true,
    }))
}

pub(crate) fn database_store(id: &str) -> LiteLlmManagedVectorStore {
    decoded(serde_json::json!({
        "vector_store_id": id,
        "custom_llm_provider": "bedrock",
        "vector_store_description": "from database",
    }))
}

pub(crate) fn labelled_store(id: &str, description: &str) -> LiteLlmManagedVectorStore {
    let mut store = store(id);
    store.vector_store_description = Some(description.to_owned());
    store.litellm_credential_name = Some("stored-credential".to_owned());
    store
}

pub(crate) fn named_store(id: &str, credential_name: &str) -> LiteLlmManagedVectorStore {
    decoded(serde_json::json!({
        "vector_store_id": id,
        "custom_llm_provider": "openai",
        "litellm_credential_name": credential_name,
    }))
}

pub(crate) fn inline_key_store(id: &str) -> LiteLlmManagedVectorStore {
    decoded(serde_json::json!({
        "vector_store_id": id,
        "custom_llm_provider": "openai",
        "litellm_params": {"api_key": "sk-inline-store-key"},
    }))
}

pub(crate) fn index(id: &str, name: &str) -> LiteLlmManagedVectorStoreIndex {
    LiteLlmManagedVectorStoreIndex {
        id: id.to_owned(),
        index_name: name.to_owned(),
        litellm_params: IndexCreateLiteLlmParams {
            vector_store_index: format!("idx-{id}"),
            vector_store_name: name.to_owned(),
        },
        index_info: None,
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

#[derive(Clone, Default)]
pub(crate) struct ScriptedDatabase {
    state: Arc<Mutex<DatabaseState>>,
}

#[derive(Default)]
struct DatabaseState {
    stores: Vec<LiteLlmManagedVectorStore>,
    indexes: Vec<LiteLlmManagedVectorStoreIndex>,
    failing: bool,
    calls: Vec<&'static str>,
}

type DatabaseRows = (
    Vec<LiteLlmManagedVectorStore>,
    Vec<LiteLlmManagedVectorStoreIndex>,
);

impl ScriptedDatabase {
    fn setting<Change>(change: Change) -> Self
    where
        Change: FnOnce(&mut DatabaseState),
    {
        let mut state = DatabaseState::default();
        change(&mut state);
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(crate) fn with_stores(stores: Vec<LiteLlmManagedVectorStore>) -> Self {
        Self::setting(|state| state.stores = stores)
    }

    pub(crate) fn with_indexes(indexes: Vec<LiteLlmManagedVectorStoreIndex>) -> Self {
        Self::setting(|state| state.indexes = indexes)
    }

    pub(crate) fn failing() -> Self {
        Self::setting(|state| state.failing = true)
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.calls.clone()
    }

    fn read(&self, table: &'static str) -> Result<DatabaseRows, TableError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.calls.push(table);
        let rows = (state.stores.clone(), state.indexes.clone());
        let failing = state.failing;
        drop(state);
        if failing {
            return Err(TableError::Publish {
                message: "database read failed".to_owned(),
            });
        }
        Ok(rows)
    }
}

impl VectorStoreDatabase for ScriptedDatabase {
    fn stores(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStore>, TableError>> + Send {
        ready(self.read("stores").map(|rows| rows.0))
    }

    fn indexes(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStoreIndex>, TableError>> + Send {
        ready(self.read("indexes").map(|rows| rows.1))
    }
}

pub(crate) struct RecordingCredentials {
    values: BTreeMap<String, JsonObject>,
    names: Mutex<Vec<String>>,
}

impl RecordingCredentials {
    pub(crate) fn new(values: BTreeMap<String, JsonObject>) -> Self {
        Self {
            values,
            names: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn names(&self) -> Vec<String> {
        let names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        names.clone()
    }
}

impl VectorStoreCredentials for RecordingCredentials {
    fn credential_values(&self, credential_name: &str) -> JsonObject {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(credential_name.to_owned());
        self.values
            .get(credential_name)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(crate) fn messages(&self) -> Vec<(Level, String)> {
        let messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        messages.clone()
    }

    pub(crate) fn debug(&self) -> Vec<String> {
        self.at(Level::DEBUG)
    }

    fn at(&self, level: Level) -> Vec<String> {
        self.messages()
            .into_iter()
            .filter(|(seen, _)| *seen == level)
            .map(|(_, message)| message)
            .collect()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, field.0));
    }
}

pub(crate) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let guard = set_default(Registry::default().with(logs.clone()));
    (logs, guard)
}
