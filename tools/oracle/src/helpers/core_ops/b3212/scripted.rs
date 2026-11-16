use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::{SkillRepository, SkillWrite};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::managed_resource_rows::wire::RowTimestamp;
use betterlite_db::postgres_store::errors::DbError;
use serde_json::{Value, json};

use super::{Input, RowFields};
use betterlite_oracle_helper_api::error::HelperError;

const CUSTOM_SOURCE: &str = "custom";

fn timestamp(text: &str) -> Result<RowTimestamp, Box<HelperError>> {
    let wire = Value::String(text.to_owned());
    let parsed = serde_json::from_value(wire);
    parsed.map_err(|error| super::invalid_input(error.to_string()))
}

fn wire_row(fields: &RowFields, now: &str) -> Result<SkillTable, Box<HelperError>> {
    let created_at = timestamp(fields.created_at.as_deref().unwrap_or(now))?;
    let updated_at = timestamp(fields.updated_at.as_deref().unwrap_or(now))?;
    Ok(SkillTable {
        skill_id: fields.skill_id.clone(),
        display_title: fields.display_title.clone(),
        description: fields.description.clone(),
        instructions: fields.instructions.clone(),
        source: fields
            .source
            .clone()
            .unwrap_or_else(|| CUSTOM_SOURCE.to_owned()),
        latest_version: fields.latest_version.clone(),
        file_content: fields.file_content.clone(),
        file_name: fields.file_name.clone(),
        file_type: fields.file_type.clone(),
        metadata: fields.metadata.clone(),
        created_at: Some(created_at),
        created_by: fields.created_by.clone(),
        updated_at: Some(updated_at),
        updated_by: fields.updated_by.clone(),
    })
}

struct State {
    available: bool,
    effects: Vec<Value>,
    rows: BTreeMap<String, SkillTable>,
    page: Vec<SkillTable>,
}

#[derive(Clone)]
pub(super) struct Scripted {
    state: Arc<Mutex<State>>,
}

impl Scripted {
    pub(super) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let mut rows = BTreeMap::new();
        for fields in &input.rows {
            rows.insert(fields.skill_id.clone(), wire_row(fields, &input.now)?);
        }
        let mut page = Vec::with_capacity(input.page.len());
        for fields in &input.page {
            page.push(wire_row(fields, &input.now)?);
        }
        let state = State {
            available: true,
            effects: Vec::new(),
            rows,
            page,
        };
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub(super) fn set_available(&self, available: bool) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available = available;
    }

    pub(super) fn response(&self, steps: &[Value]) -> Value {
        let effects = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .clone();
        json!({"steps": steps, "effects": effects})
    }

    fn push_effect(&self, effect: Value) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push(effect);
    }

    fn found_row(&self, skill_id: &str) -> Option<SkillTable> {
        self.push_effect(json!({"kind": "find_unique", "where": {"skill_id": skill_id}}));
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .get(skill_id)
            .cloned()
    }

    fn found_page(&self, page: SkillPage, created_by: Option<Vec<String>>) -> Vec<SkillTable> {
        let effect = json!({
            "kind": "find_many",
            "take": page.limit,
            "skip": page.offset,
            "order": {"created_at": "desc"},
            "where": created_by.map(|scopes| json!({"created_by": {"in": scopes}})),
        });
        self.push_effect(effect);
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .page
            .clone()
    }
}

impl SkillRepository for Scripted {
    fn availability(&self) -> Result<(), TableError> {
        let available = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .available;
        if available {
            return Ok(());
        }
        Err(TableError::Database(DbError::NoDbConnected))
    }

    fn create(
        &self,
        _write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "create is not scripted".to_owned(),
        }))
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

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(TableError::Publish {
            message: "delete is not scripted".to_owned(),
        }))
    }
}
