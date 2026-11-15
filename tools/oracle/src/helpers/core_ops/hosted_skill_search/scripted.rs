use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::skills::store::SkillPage;
use betterlite_core::skills::store::repository::{SkillRepository, SkillWrite};
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_semantic::{EmbedError, Embedder, Vector};
use serde_json::{Value, json};

use super::input::Input;
use super::values::{invalid_input, skill_rows};
use crate::helpers::HelperError;

pub(super) const TEST_SKILL_ID: &str = "litellm_skill_test";
pub(super) const CACHE_CLOCK: f64 = 1000.0;
const HTTP_STATUS_CODE: u16 = 429;
const HTTP_DETAIL: &str = "slow down";
const VALUE_FAILURE: &str = "boom";
const EMBED_KIND: &str = "embed";
const FIND_MANY_KIND: &str = "find_many";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    Value,
    Http,
}

#[derive(Clone, Default)]
pub(super) struct Effects {
    entries: Arc<Mutex<Vec<Value>>>,
}

impl Effects {
    pub(super) fn push(&self, entry: Value) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn take(&self) -> Vec<Value> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *entries)
    }
}

pub(super) struct ScriptedEmbedder {
    vectors: BTreeMap<String, Vector>,
    fault: Option<Fault>,
    effects: Effects,
}

impl ScriptedEmbedder {
    pub(super) fn new(input: &Input, effects: &Effects) -> Result<Self, Box<HelperError>> {
        let fault = match input.fault.as_deref() {
            None => None,
            Some("value") => Some(Fault::Value),
            Some("http") => Some(Fault::Http),
            Some(other) => return Err(invalid_input(format!("unknown fault: {other}"))),
        };
        Ok(Self {
            vectors: input.vectors.clone(),
            fault,
            effects: effects.clone(),
        })
    }
}

#[async_trait]
impl Embedder for ScriptedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vector>, EmbedError> {
        self.effects
            .push(json!({"kind": EMBED_KIND, "texts": texts}));
        match self.fault {
            Some(Fault::Value) => return Err(EmbedError::Other(VALUE_FAILURE.to_owned())),
            Some(Fault::Http) => {
                return Err(EmbedError::Http {
                    status_code: HTTP_STATUS_CODE,
                    detail: HTTP_DETAIL.to_owned(),
                });
            }
            None => {}
        }
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .ok_or_else(|| EmbedError::Other(format!("no vector for {text}")))
            })
            .collect()
    }
}

pub(super) struct ScriptedRepository {
    rows: Vec<SkillTable>,
    effects: Effects,
}

impl ScriptedRepository {
    pub(super) fn new(input: &Input, effects: &Effects) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            rows: skill_rows(&input.rows)?,
            effects: effects.clone(),
        })
    }
}

fn unscripted(action: &str) -> TableError {
    TableError::Publish {
        message: format!("the scripted repository has no {action}"),
    }
}

fn visible_rows(
    rows: &[SkillTable],
    page: SkillPage,
    created_by: Option<&[String]>,
) -> Vec<SkillTable> {
    let offset = usize::try_from(page.offset).unwrap_or(0);
    let limit = usize::try_from(page.limit).unwrap_or(usize::MAX);
    rows.iter()
        .filter(|row| {
            created_by.is_none_or(|scopes| {
                row.created_by
                    .as_deref()
                    .is_some_and(|owner| scopes.iter().any(|scope| scope.as_str() == owner))
            })
        })
        .skip(offset)
        .take(limit)
        .cloned()
        .collect()
}

impl SkillRepository for ScriptedRepository {
    fn availability(&self) -> Result<(), TableError> {
        Ok(())
    }

    fn create(
        &self,
        _write: SkillWrite,
    ) -> impl Future<Output = Result<SkillTable, TableError>> + Send {
        ready(Err(unscripted("create")))
    }

    fn find_unique(
        &self,
        _skill_id: &str,
    ) -> impl Future<Output = Result<Option<SkillTable>, TableError>> + Send {
        ready(Err(unscripted("find_unique")))
    }

    fn find_many(
        &self,
        page: SkillPage,
        created_by: Option<Vec<String>>,
    ) -> impl Future<Output = Result<Vec<SkillTable>, TableError>> + Send {
        self.effects.push(json!({
            "kind": FIND_MANY_KIND,
            "take": page.limit,
            "skip": page.offset,
            "order": {"created_at": "desc"},
            "where": created_by
                .as_ref()
                .map(|scopes| json!({"created_by": {"in": scopes}})),
        }));
        let rows = visible_rows(&self.rows, page, created_by.as_deref());
        ready(Ok(rows))
    }

    fn delete(&self, _skill_id: &str) -> impl Future<Output = Result<(), TableError>> + Send {
        ready(Err(unscripted("delete")))
    }
}
