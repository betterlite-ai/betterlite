use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_server::api::vector_management_boundaries::rows::VectorStoreRows;
use betterlite_server::api::vector_management_write::rows::{
    NewVectorStoreRecord, VectorStoreChanges, VectorStoreWrites,
};
use serde_json::Value;

// the fake database clock, the same values the Python helper's table stamps
const CREATED_AT: &str = "2024-01-02T03:04:05.000";
const UPDATED_AT: &str = "2024-02-03T04:05:06.000";

type Found = Result<Option<LiteLlmManagedVectorStore>, TableError>;
type Stored = Result<LiteLlmManagedVectorStore, TableError>;
type Rows<'guard> = MutexGuard<'guard, DbState>;
// an update field: the outer Option is presence and the inner one an explicit null, as exclude_unset keeps them apart
type FieldChange<T> = Option<Option<T>>;

// the rows the management writes change, with every call recorded like the table's
#[derive(Clone)]
pub(super) struct FakeDatabase {
    pub(super) inner: Arc<Mutex<DbState>>,
    events: Arc<Mutex<Vec<String>>>,
}

pub(super) struct DbState {
    pub(super) rows: Vec<LiteLlmManagedVectorStore>,
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

    fn note(&self, entry: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    // a call's event, then the rows it reads or changes
    fn begin(&self, entry: &str) -> Rows<'_> {
        self.note(entry.to_owned());
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn find(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        self.begin("db:find")
            .rows
            .iter()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .cloned()
    }

    fn insert(&self, record: &NewVectorStoreRecord) -> LiteLlmManagedVectorStore {
        let store = record_to_store(record);
        self.begin("db:create").rows.push(store.clone());
        store
    }

    fn replace(
        &self,
        vector_store_id: &str,
        changes: &VectorStoreChanges,
    ) -> Option<LiteLlmManagedVectorStore> {
        let mut state = self.begin("db:update");
        state
            .rows
            .iter_mut()
            .find(|row| row.vector_store_id.as_deref() == Some(vector_store_id))
            .map(|row| {
                apply_changes(row, changes);
                row.clone()
            })
    }

    fn take(&self, vector_store_id: &str) -> Option<LiteLlmManagedVectorStore> {
        let mut state = self.begin("db:delete");
        let position = state
            .rows
            .iter()
            .position(|row| row.vector_store_id.as_deref() == Some(vector_store_id));
        let removed = position.map(|index| state.rows.remove(index));
        drop(state);
        removed
    }
}

impl VectorStoreRows for FakeDatabase {
    fn find_unique(&self, vector_store_id: &str) -> impl Future<Output = Found> + Send {
        ready(Ok(self.find(vector_store_id)))
    }
}

impl VectorStoreWrites for FakeDatabase {
    fn create(&self, record: NewVectorStoreRecord) -> impl Future<Output = Stored> + Send {
        ready(Ok(self.insert(&record)))
    }

    fn update(
        &self,
        vector_store_id: &str,
        changes: VectorStoreChanges,
    ) -> impl Future<Output = Found> + Send {
        ready(Ok(self.replace(vector_store_id, &changes)))
    }

    fn delete(&self, vector_store_id: &str) -> impl Future<Output = Found> + Send {
        ready(Ok(self.take(vector_store_id)))
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
        changes.custom_llm_provider.as_ref().map(Option::as_deref),
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_name,
        changes.vector_store_name.as_ref().map(Option::as_deref),
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_description,
        changes
            .vector_store_description
            .as_ref()
            .map(Option::as_deref),
        |text: &str| text.to_owned(),
    );
    apply(
        &mut row.vector_store_metadata,
        changes.vector_store_metadata.as_ref().map(Option::as_deref),
        |text: &str| Value::String(text.to_owned()),
    );
    row.updated_at = Some(String::from(UPDATED_AT));
}

fn apply<T, F>(field: &mut Option<T>, change: FieldChange<&str>, convert: F)
where
    F: Fn(&str) -> T,
{
    match change {
        None => {}
        Some(None) => *field = None,
        Some(Some(text)) => *field = Some(convert(text)),
    }
}
