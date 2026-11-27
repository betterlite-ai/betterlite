use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_server::api::vector_management_boundaries::rows::VectorStoreRows;
use betterlite_server::api::vector_management_write::rows::{
    FieldChange, NewVectorStoreRecord, VectorStoreChanges, VectorStoreWrites,
};
use serde_json::Value;

// the fake database clock, the same values the Python helper's table stamps
const CREATED_AT: &str = "2024-01-02T03:04:05.000";
const UPDATED_AT: &str = "2024-02-03T04:05:06.000";

// the rows the management writes change, with every call recorded like the registry's
#[derive(Clone)]
pub(super) struct FakeDatabase {
    inner: Arc<Mutex<DbState>>,
    events: Arc<Mutex<Vec<String>>>,
}

struct DbState {
    rows: Vec<LiteLlmManagedVectorStore>,
}

impl FakeDatabase {
    pub(super) fn new(
        rows: Vec<LiteLlmManagedVectorStore>,
        events: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(DbState { rows })),
            events,
        }
    }

    pub(super) fn rows(&self) -> Vec<LiteLlmManagedVectorStore> {
        self.state().rows.clone()
    }

    fn state(&self) -> MutexGuard<'_, DbState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn note(&self, entry: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }
}

impl VectorStoreRows for FakeDatabase {
    fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send {
        self.note(format!("db:find:{vector_store_id}"));
        let found = self
            .state()
            .rows
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned();
        ready(Ok(found))
    }
}

impl VectorStoreWrites for FakeDatabase {
    fn create(
        &self,
        record: NewVectorStoreRecord,
    ) -> impl Future<Output = Result<LiteLlmManagedVectorStore, TableError>> + Send {
        self.note(format!("db:create:{}", record.vector_store_id));
        let store = record_to_store(&record);
        self.state().rows.push(store.clone());
        ready(Ok(store))
    }

    fn update(
        &self,
        vector_store_id: &str,
        changes: VectorStoreChanges,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send {
        self.note(format!("db:update:{vector_store_id}"));
        let mut state = self.state();
        let updated = state
            .rows
            .iter_mut()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .map(|row| {
                apply_changes(row, &changes);
                row.clone()
            });
        ready(Ok(updated))
    }

    fn delete(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<LiteLlmManagedVectorStore>, TableError>> + Send {
        self.note(format!("db:delete:{vector_store_id}"));
        let mut state = self.state();
        let removed = state
            .rows
            .iter()
            .position(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .map(|position| state.rows.remove(position));
        ready(Ok(removed))
    }
}

// the row a create returns: what Prisma would hold after the insert, with the table's timestamps
fn record_to_store(record: &NewVectorStoreRecord) -> LiteLlmManagedVectorStore {
    LiteLlmManagedVectorStore {
        vector_store_id: Some(record.vector_store_id.clone()),
        custom_llm_provider: Some(record.custom_llm_provider.clone()),
        vector_store_name: record.vector_store_name.clone(),
        vector_store_description: record.vector_store_description.clone(),
        vector_store_metadata: record
            .vector_store_metadata
            .as_ref()
            .map(|text| Value::String(text.clone())),
        created_at: Some(String::from(CREATED_AT)),
        updated_at: Some(String::from(UPDATED_AT)),
        litellm_credential_name: record.litellm_credential_name.clone(),
        litellm_params: Some(Value::String(record.litellm_params.clone())),
        team_id: record.team_id.clone(),
        user_id: record.user_id.clone(),
        is_config: None,
        extra: BTreeMap::new(),
    }
}

fn apply_changes(row: &mut LiteLlmManagedVectorStore, changes: &VectorStoreChanges) {
    apply(
        &mut row.custom_llm_provider,
        &changes.custom_llm_provider,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_name,
        &changes.vector_store_name,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_description,
        &changes.vector_store_description,
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_metadata,
        &changes.vector_store_metadata,
        |text: &str| Value::String(text.to_owned()),
    );
    row.updated_at = Some(String::from(UPDATED_AT));
}

fn apply<T>(field: &mut Option<T>, change: &FieldChange<String>, convert: impl Fn(&str) -> T) {
    match change {
        FieldChange::Omitted => {}
        FieldChange::Null => *field = None,
        FieldChange::Value(text) => *field = Some(convert(text)),
    }
}
