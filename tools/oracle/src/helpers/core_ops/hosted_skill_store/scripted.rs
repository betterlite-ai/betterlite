use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::{SkillRepository, SkillWrite};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use serde_json::{Map, Value, json};

use super::RowFields;

const CUSTOM_SOURCE: &str = "custom";

fn wire_row(fields: &RowFields, now: &str) -> Result<SkillTable, TableError> {
    let value = json!({
        "skill_id": &fields.skill_id,
        "display_title": &fields.display_title,
        "description": &fields.description,
        "instructions": &fields.instructions,
        "source": fields.source.as_deref().unwrap_or(CUSTOM_SOURCE),
        "latest_version": &fields.latest_version,
        "file_content": &fields.file_content,
        "file_name": &fields.file_name,
        "file_type": &fields.file_type,
        "metadata": &fields.metadata,
        "created_at": fields.created_at.as_deref().unwrap_or(now),
        "created_by": &fields.created_by,
        "updated_at": fields.updated_at.as_deref().unwrap_or(now),
        "updated_by": &fields.updated_by,
    });
    serde_json::from_value(value).map_err(TableError::Row)
}

struct State {
    now: String,
    effects: Vec<Value>,
    rows: BTreeMap<String, SkillTable>,
    page: Vec<SkillTable>,
    faults: BTreeMap<String, bool>,
}

#[derive(Clone)]
pub(super) struct Scripted {
    state: Arc<Mutex<State>>,
}

impl Scripted {
    pub(super) fn new(input: &super::Input) -> Result<Self, Box<crate::helpers::HelperError>> {
        let mut rows = BTreeMap::new();
        for fields in &input.rows {
            let row = wire_row(fields, &input.now)
                .map_err(|error| super::invalid_input(error.to_string()))?;
            rows.insert(fields.skill_id.clone(), row);
        }
        let mut page = Vec::with_capacity(input.page.len());
        for fields in &input.page {
            page.push(
                wire_row(fields, &input.now)
                    .map_err(|error| super::invalid_input(error.to_string()))?,
            );
        }
        let state = Mutex::new(State {
            now: input.now.clone(),
            effects: Vec::new(),
            rows,
            page,
            faults: input.faults.clone(),
        });
        Ok(Self {
            state: Arc::new(state),
        })
    }

    pub(super) fn response(&self, steps: &[Value], warnings: &[Value]) -> Value {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        json!({
            "steps": steps,
            "effects": state.effects,
            "warnings": warnings,
        })
    }

    fn failure(&self, kind: &str) -> Result<(), TableError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let faulted = state.faults.get(kind).copied().unwrap_or(false);
        if !faulted {
            return Ok(());
        }
        Err(TableError::Publish {
            message: format!("the scripted repository failed the {kind}"),
        })
    }

    fn record_create(&self, write: &SkillWrite, metadata: Option<&Value>) {
        let mut entries = vec![
            ("skill_id", json!(&write.skill_id)),
            ("display_title", json!(&write.display_title)),
            ("description", json!(&write.description)),
            ("instructions", json!(&write.instructions)),
            ("source", json!(&write.source)),
            ("created_by", json!(&write.created_by)),
            ("updated_by", json!(&write.updated_by)),
        ];
        if let Some(value) = metadata {
            entries.push(("metadata", value.clone()));
        }
        if let Some(text) = &write.file_content {
            entries.push(("file_content", json!(text)));
        }
        if let Some(name) = &write.file_name {
            entries.push(("file_name", json!(name)));
        }
        if let Some(kind) = &write.file_type {
            entries.push(("file_type", json!(kind)));
        }
        let data = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Map<String, Value>>();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push(json!({"kind": "create", "data": data}));
    }
}

impl SkillRepository for Scripted {
    fn availability(&self) -> Result<(), TableError> {
        Ok(())
    }

    async fn create(&self, write: SkillWrite) -> Result<SkillTable, TableError> {
        let metadata = write
            .metadata
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(TableError::Row)?;
        self.record_create(&write, metadata.as_ref());
        self.failure("create")?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let stored = metadata.unwrap_or_else(|| Value::Object(Map::new()));
        let row = serde_json::from_value(json!({
            "skill_id": &write.skill_id,
            "display_title": &write.display_title,
            "description": &write.description,
            "instructions": &write.instructions,
            "source": &write.source,
            "latest_version": null,
            "file_content": &write.file_content,
            "file_name": &write.file_name,
            "file_type": &write.file_type,
            "metadata": stored,
            "created_at": &state.now,
            "created_by": &write.created_by,
            "updated_at": &state.now,
            "updated_by": &write.updated_by,
        }))
        .map_err(TableError::Row)?;
        state.rows.insert(write.skill_id.clone(), row.clone());
        Ok(row)
    }

    async fn find_unique(&self, skill_id: &str) -> Result<Option<SkillTable>, TableError> {
        let found = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state
                .effects
                .push(json!({"kind": "find_unique", "where": {"skill_id": skill_id}}));
            state.rows.get(skill_id).cloned()
        };
        self.failure("find_unique")?;
        Ok(found)
    }

    async fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> Result<Vec<SkillTable>, TableError> {
        let rows = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.effects.push(json!({
                "kind": "find_many",
                "take": page.limit,
                "skip": page.offset,
                "order": {"created_at": "desc"},
                "where": created_by.map(|scopes| json!({"created_by": {"in": scopes}})),
            }));
            state.page.clone()
        };
        self.failure("find_many")?;
        Ok(rows)
    }

    async fn delete(&self, skill_id: &str) -> Result<(), TableError> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .effects
            .push(json!({"kind": "delete", "where": {"skill_id": skill_id}}));
        self.failure("delete")?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .remove(skill_id);
        Ok(())
    }
}
