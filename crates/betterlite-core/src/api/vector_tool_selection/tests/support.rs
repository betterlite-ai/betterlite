use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::pydantic::LiteLlmManagedVectorStoreIndex;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::api::vector_store_registry::repository::VectorStoreDatabase;
use crate::api::vector_store_registry::store::VectorStoreRegistry;

use super::super::database::VectorStoreSelectionDatabase;

pub(crate) fn registry(stores: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    VectorStoreRegistry {
        stores,
        clock: Arc::new(Utc::now),
        credentials: Arc::new(CredentialSnapshot::default()),
    }
}

pub(crate) fn copied(registry: &VectorStoreRegistry) -> VectorStoreRegistry {
    VectorStoreRegistry {
        stores: registry.stores.clone(),
        clock: Arc::clone(&registry.clock),
        credentials: Arc::clone(&registry.credentials),
    }
}

pub(crate) fn store(id: &str) -> LiteLlmManagedVectorStore {
    LiteLlmManagedVectorStore {
        vector_store_id: Some(id.to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        vector_store_name: None,
        vector_store_description: None,
        vector_store_metadata: None,
        created_at: None,
        updated_at: None,
        litellm_credential_name: None,
        litellm_params: None,
        team_id: None,
        user_id: None,
        is_config: None,
        extra: BTreeMap::new(),
    }
}

pub(crate) fn with_params(
    mut store: LiteLlmManagedVectorStore,
    params: JsonObject,
) -> LiteLlmManagedVectorStore {
    store.litellm_params = Some(serde_json::to_value(Json::Object(params)).expect("params encode"));
    store
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    let mut fields = JsonObject::new();
    for (name, value) in entries {
        fields.insert(name.to_owned(), value);
    }
    fields
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn list(ids: &[&str]) -> Json {
    Json::List(ids.iter().map(|id| text(id)).collect())
}

pub(crate) fn read_failure() -> TableError {
    TableError::Publish {
        message: "database read failed".to_owned(),
    }
}

pub(crate) struct ScriptedDatabase {
    rows: Vec<LiteLlmManagedVectorStore>,
    verifiable: BTreeMap<String, LiteLlmManagedVectorStore>,
    verification_fails: bool,
    read_fails: bool,
    recorded: Mutex<Vec<String>>,
}

impl ScriptedDatabase {
    pub(crate) fn without_rows() -> Self {
        Self::with_rows(Vec::new())
    }

    pub(crate) fn with_rows(rows: Vec<LiteLlmManagedVectorStore>) -> Self {
        Self {
            rows,
            verifiable: BTreeMap::new(),
            verification_fails: false,
            read_fails: false,
            recorded: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub(crate) fn with_verifiable(mut self, store: LiteLlmManagedVectorStore) -> Self {
        if let Some(id) = &store.vector_store_id {
            self.verifiable.insert(id.clone(), store);
        }
        self
    }

    #[must_use]
    pub(crate) fn with_verification_failure(mut self) -> Self {
        self.verification_fails = true;
        self
    }

    #[must_use]
    pub(crate) fn with_read_failure(mut self) -> Self {
        self.read_fails = true;
        self
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        let recorded = self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.clone()
    }

    fn record(&self, call: String) {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn find_many(&self) -> Result<Vec<LiteLlmManagedVectorStore>, TableError> {
        self.record("find_many".to_owned());
        if self.read_fails {
            return Err(read_failure());
        }
        Ok(self.rows.clone())
    }

    fn find_one(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, TableError> {
        self.record(format!("find_unique:{vector_store_id}"));
        if self.verification_fails {
            return Err(read_failure());
        }
        Ok(self.verifiable.get(vector_store_id).cloned())
    }
}

impl VectorStoreDatabase for ScriptedDatabase {
    fn stores(
        &self,
    ) -> impl Future<Output = Result<Vec<LiteLlmManagedVectorStore>, TableError>> + Send {
        ready(self.find_many())
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
        ready(self.find_one(vector_store_id))
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl CapturedLogs {
    pub(crate) fn install() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let writer = logs.clone();
        let subscriber = fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let guard = set_default(subscriber);
        (logs, guard)
    }

    pub(crate) fn text(&self) -> String {
        let buffer = self.buffer.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(buffer.clone()).expect("the log output is UTF-8")
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
