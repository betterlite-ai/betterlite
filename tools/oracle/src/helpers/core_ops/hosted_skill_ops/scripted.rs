use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::{SkillRepository, SkillWrite};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use serde_json::{Value, json};

use super::{Input, invalid_input};

struct State {
    now: RowTimestamp,
    effects: Vec<Value>,
    rows: BTreeMap<String, SkillTable>,
    page: Vec<SkillTable>,
}

#[derive(Clone)]
pub(super) struct Scripted {
    state: Arc<Mutex<State>>,
}

impl Scripted {
    pub(super) fn new(input: &Input) -> Result<Self, Box<crate::helpers::HelperError>> {
        let now: RowTimestamp = serde_json::from_value(Value::String(input.now.clone()))
            .map_err(|error| invalid_input(error.to_string()))?;
        let rows = input
            .rows
            .iter()
            .map(|row| (row.skill_id.clone(), row.clone()))
            .collect();
        let state = Mutex::new(State {
            now,
            effects: Vec::new(),
            rows,
            page: input.page.clone(),
        });
        Ok(Self {
            state: Arc::new(state),
        })
    }

    pub(super) fn response(&self, steps: &[Value]) -> Value {
        let effects = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .clone();
        json!({
            "steps": steps,
            "effects": effects,
        })
    }

    fn record_create(&self, write: &SkillWrite, metadata: Option<&Value>) {
        let mut data = serde_json::Map::new();
        data.insert("skill_id".to_owned(), json!(&write.skill_id));
        data.insert("display_title".to_owned(), json!(&write.display_title));
        data.insert("description".to_owned(), json!(&write.description));
        data.insert("instructions".to_owned(), json!(&write.instructions));
        data.insert("source".to_owned(), json!(&write.source));
        data.insert("created_by".to_owned(), json!(&write.created_by));
        data.insert("updated_by".to_owned(), json!(&write.updated_by));
        if let Some(value) = metadata {
            data.insert("metadata".to_owned(), value.clone());
        }
        if let Some(text) = &write.file_content {
            data.insert("file_content".to_owned(), json!(text));
        }
        if let Some(name) = &write.file_name {
            data.insert("file_name".to_owned(), json!(name));
        }
        if let Some(kind) = &write.file_type {
            data.insert("file_type".to_owned(), json!(kind));
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push(json!({"kind": "create", "data": data}));
    }

    fn row_for(
        &self,
        write: &SkillWrite,
        metadata: Option<Value>,
    ) -> Result<SkillTable, TableError> {
        let metadata: Option<BTreeMap<String, JsonData>> = metadata
            .map(serde_json::from_value)
            .transpose()
            .map_err(TableError::Row)?;
        let now = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now;
        Ok(SkillTable {
            skill_id: write.skill_id.clone(),
            display_title: write.display_title.clone(),
            description: write.description.clone(),
            instructions: write.instructions.clone(),
            source: write.source.clone(),
            latest_version: None,
            file_content: write.file_content.clone(),
            file_name: write.file_name.clone(),
            file_type: write.file_type.clone(),
            metadata,
            created_at: Some(now),
            created_by: Some(write.created_by.clone()),
            updated_at: Some(now),
            updated_by: Some(write.updated_by.clone()),
        })
    }

    fn created_row(&self, write: &SkillWrite) -> Result<SkillTable, TableError> {
        let metadata: Option<Value> = write
            .metadata
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(TableError::Row)?;
        self.record_create(write, metadata.as_ref());
        let row = self.row_for(write, metadata)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .insert(write.skill_id.clone(), row.clone());
        Ok(row)
    }

    fn found_row(&self, skill_id: &str) -> Option<SkillTable> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .effects
            .push(json!({"kind": "find_unique", "where": {"skill_id": skill_id}}));
        state.rows.get(skill_id).cloned()
    }

    fn found_page(&self, page: SkillPage, created_by: Option<Vec<String>>) -> Vec<SkillTable> {
        let matched = created_by.map(|scopes| json!({"created_by": {"in": scopes}}));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.effects.push(json!({
            "kind": "find_many",
            "take": page.limit,
            "skip": page.offset,
            "order": {"created_at": "desc"},
            "where": matched,
        }));
        state.page.clone()
    }

    fn deleted_row(&self, skill_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .effects
            .push(json!({"kind": "delete", "where": {"skill_id": skill_id}}));
        state.rows.remove(skill_id);
    }
}

impl SkillRepository for Scripted {
    fn availability(&self) -> Result<(), TableError> {
        Ok(())
    }

    fn create(
        &self,
        write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(self.created_row(&write))
    }

    fn find_unique(
        &self,
        skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        ready(Ok(self.found_row(skill_id)))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        ready(Ok(self.found_page(page, created_by)))
    }

    fn delete(&self, skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready({
            self.deleted_row(skill_id);
            Ok(())
        })
    }
}
