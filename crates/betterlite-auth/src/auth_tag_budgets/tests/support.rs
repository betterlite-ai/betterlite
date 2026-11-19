use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::errors::DbError as RepositoryFault;
use betterlite_models::decisions::Json;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::payload::PayloadError;
use crate::auth_read::table::AuthReadError;
use crate::auth_resource_grants::rows::RowError;
use crate::auth_tag_budgets::cache::TagBudgetCache;
use crate::auth_tag_budgets::groups::ModelAccessGroupBudget;
use crate::auth_tag_budgets::rows::{
    GroupRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure, TagRows,
};

pub(super) const TTL: f64 = 60.0;
pub(super) const TAG_REGISTRY_KEY: &str = "tag_registry";
pub(super) const GROUP_REGISTRY_KEY: &str = "model_access_group_registry";

pub(super) const TAG_A: &str = r#"{"tag_name":"tag-a","models":["m1"],"spend":1.5}"#;
pub(super) const TAG_B: &str = r#"{"tag_name":"tag-b","models":[],"spend":0}"#;
pub(super) const TAG_C: &str = r#"{"tag_name":"tag-c","models":["m3"],"spend":2.25}"#;

pub(super) const GROUP_A: &str = r#"{"access_group_name":"group-a","spend":3.5}"#;
pub(super) const GROUP_B: &str = r#"{"access_group_name":"group-b","spend":0}"#;
pub(super) const GROUP_C: &str = r#"{"access_group_name":"group-c","spend":7}"#;
pub(super) const GROUP_ZERO_BUDGET: &str = r#"{"access_group_name":"group-a","spend":0,"litellm_budget_table":{"budget_id":"b1","max_budget":0}}"#;

pub(super) fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

pub(super) fn tag_key(name: &str) -> String {
    CacheKey::Tag(name).render()
}

pub(super) fn group_key(name: &str) -> String {
    CacheKey::ModelAccessGroup(name).render()
}

pub(super) fn registry_names(names: &[&str]) -> Json {
    Json::List(
        names
            .iter()
            .map(|name| Json::Text((*name).to_owned()))
            .collect(),
    )
}

pub(super) fn tag(text: &str) -> TagTable {
    serde_json::from_str(text).expect("the tag fixture decodes")
}

pub(super) fn group_row(text: &str) -> ModelAccessGroupBudgetRow {
    serde_json::from_str(text).expect("the group row fixture decodes")
}

pub(super) fn group_budget(text: &str) -> ModelAccessGroupBudget {
    serde_json::from_str(text).expect("the group budget fixture decodes")
}

pub(super) fn named_tags(count: usize) -> Vec<TagTable> {
    (0..count)
        .map(|index| {
            let wire =
                serde_json::json!({"tag_name": format!("tag-{index}"), "models": [], "spend": 0});
            serde_json::from_value(wire).expect("the generated tag row decodes")
        })
        .collect()
}

pub(super) fn named_group_rows(count: usize) -> Vec<ModelAccessGroupBudgetRow> {
    (0..count)
        .map(|index| {
            let wire =
                serde_json::json!({"access_group_name": format!("group-{index}"), "spend": 0});
            serde_json::from_value(wire).expect("the generated group row decodes")
        })
        .collect()
}

fn no_database() -> DbError {
    DbError {
        error_type: ErrorType::NoDbConnection,
        message: "No DB Connected".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }
}

pub(super) fn query_failure() -> TagBudgetRowsFailure {
    TagBudgetRowsFailure {
        source: Box::new(AuthReadError::Lookup(
            DbLookupError::Database(no_database()),
        )),
    }
}

pub(super) fn decode_failure() -> TagBudgetRowsFailure {
    let record = serde_json::from_str::<i64>("not a number").expect_err("the invalid json");
    TagBudgetRowsFailure {
        source: Box::new(AuthReadError::Repository(RepositoryFault::InvalidRecord {
            source: RowError::Record(record),
        })),
    }
}

pub(super) struct RecordingCache {
    tags: BTreeMap<String, TagTable>,
    groups: BTreeMap<String, ModelAccessGroupBudget>,
    registries: BTreeMap<String, Json>,
    once_tag: Mutex<Option<(String, TagTable)>>,
    once_group: Mutex<Option<(String, ModelAccessGroupBudget)>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<(String, f64)>>,
    ttl: f64,
    fail_tag_read: bool,
    fail_group_read: bool,
    fail_registry_read: bool,
    fail_write_after: Option<usize>,
}

impl RecordingCache {
    pub(super) fn new(ttl: f64) -> Self {
        Self {
            tags: BTreeMap::new(),
            groups: BTreeMap::new(),
            registries: BTreeMap::new(),
            once_tag: Mutex::new(None),
            once_group: Mutex::new(None),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            ttl,
            fail_tag_read: false,
            fail_group_read: false,
            fail_registry_read: false,
            fail_write_after: None,
        }
    }

    pub(super) fn with_tag(mut self, row: TagTable) -> Self {
        self.tags.insert(tag_key(&row.tag_name), row);
        self
    }

    pub(super) fn with_tag_hit_once(mut self, row: TagTable) -> Self {
        self.once_tag = Mutex::new(Some((tag_key(&row.tag_name), row)));
        self
    }

    pub(super) fn with_group(mut self, budget: ModelAccessGroupBudget) -> Self {
        self.groups
            .insert(group_key(&budget.access_group_name), budget);
        self
    }

    pub(super) fn with_group_hit_once(mut self, budget: ModelAccessGroupBudget) -> Self {
        self.once_group = Mutex::new(Some((group_key(&budget.access_group_name), budget)));
        self
    }

    pub(super) fn with_registry(mut self, key: &str, answer: Json) -> Self {
        self.registries.insert(key.to_owned(), answer);
        self
    }

    pub(super) fn failing_tag_read(mut self) -> Self {
        self.fail_tag_read = true;
        self
    }

    pub(super) fn failing_group_read(mut self) -> Self {
        self.fail_group_read = true;
        self
    }

    pub(super) fn failing_registry_read(mut self) -> Self {
        self.fail_registry_read = true;
        self
    }

    pub(super) fn failing_writes_after(mut self, written: usize) -> Self {
        self.fail_write_after = Some(written);
        self
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads lock").clone()
    }

    pub(super) fn tagged_reads(&self, prefix: &str) -> Vec<String> {
        self.reads()
            .into_iter()
            .filter(|key| key.starts_with(prefix))
            .collect()
    }

    pub(super) fn writes(&self) -> Vec<(String, f64)> {
        self.writes.lock().expect("the cache writes lock").clone()
    }

    pub(super) fn registry_answer(&self, key: &str) -> Option<Json> {
        self.registries.get(key).cloned()
    }

    pub(super) fn prefixed_writes(&self, prefix: &str) -> Vec<(String, f64)> {
        self.writes()
            .into_iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .collect()
    }

    fn record_read(&self, key: &str) {
        self.reads
            .lock()
            .expect("the cache reads lock")
            .push(key.to_owned());
    }

    fn record_write(&self, key: &str, ttl: f64) -> Result<(), AuthCacheError> {
        let mut writes = self.writes.lock().expect("the cache writes lock");
        if self
            .fail_write_after
            .is_some_and(|allowed| writes.len() >= allowed)
        {
            return Err(AuthCacheError::Payload(Box::new(payload_error())));
        }
        writes.push((key.to_owned(), ttl));
        Ok(())
    }
}

#[async_trait]
impl TagBudgetCache for RecordingCache {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        self.record_read(key);
        if self.fail_tag_read {
            return Err(payload_error());
        }
        let mut once = self.once_tag.lock().expect("the scripted tag lock");
        if once.as_ref().is_some_and(|(once_key, _)| once_key == key) {
            let scripted = once.take();
            return Ok(scripted.map(|(_, row)| row));
        }
        Ok(self.tags.get(key).cloned())
    }

    async fn write_tag(&self, key: &str, _tag: &TagTable, ttl: f64) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl)
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        self.record_read(key);
        if self.fail_group_read {
            return Err(payload_error());
        }
        let mut once = self.once_group.lock().expect("the scripted group lock");
        if once.as_ref().is_some_and(|(once_key, _)| once_key == key) {
            let scripted = once.take();
            return Ok(scripted.map(|(_, budget)| budget));
        }
        Ok(self.groups.get(key).cloned())
    }

    async fn write_group(
        &self,
        key: &str,
        _budget: &ModelAccessGroupBudget,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl)
    }

    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        self.record_read(key);
        if self.fail_registry_read {
            return Err(AuthCacheError::Payload(Box::new(payload_error())));
        }
        Ok(self.registries.get(key).cloned())
    }

    async fn write_registry(
        &self,
        key: &str,
        _answer: &Json,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl)
    }

    fn management_ttl(&self) -> f64 {
        self.ttl
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    None,
    Query,
    Decode,
}

pub(super) struct RecordingRows {
    tags: Vec<TagTable>,
    groups: Vec<ModelAccessGroupBudgetRow>,
    tag_queries: Mutex<Vec<Vec<String>>>,
    group_queries: Mutex<Vec<Vec<String>>>,
    registry_takes: Mutex<Vec<i64>>,
    group_registry_takes: Mutex<Vec<i64>>,
    tag_failure: Failure,
    group_failure: Failure,
}

impl RecordingRows {
    pub(super) fn new(tags: Vec<TagTable>, groups: Vec<ModelAccessGroupBudgetRow>) -> Self {
        Self {
            tags,
            groups,
            tag_queries: Mutex::new(Vec::new()),
            group_queries: Mutex::new(Vec::new()),
            registry_takes: Mutex::new(Vec::new()),
            group_registry_takes: Mutex::new(Vec::new()),
            tag_failure: Failure::None,
            group_failure: Failure::None,
        }
    }

    pub(super) fn failing_tags(mut self) -> Self {
        self.tag_failure = Failure::Query;
        self
    }

    pub(super) fn undecodable_tags(mut self) -> Self {
        self.tag_failure = Failure::Decode;
        self
    }

    pub(super) fn failing_groups(mut self) -> Self {
        self.group_failure = Failure::Query;
        self
    }

    pub(super) fn undecodable_groups(mut self) -> Self {
        self.group_failure = Failure::Decode;
        self
    }

    pub(super) fn tag_queries(&self) -> Vec<Vec<String>> {
        self.tag_queries
            .lock()
            .expect("the tag queries lock")
            .clone()
    }

    pub(super) fn group_queries(&self) -> Vec<Vec<String>> {
        self.group_queries
            .lock()
            .expect("the group queries lock")
            .clone()
    }

    pub(super) fn registry_takes(&self) -> Vec<i64> {
        self.registry_takes
            .lock()
            .expect("the registry takes lock")
            .clone()
    }

    pub(super) fn group_registry_takes(&self) -> Vec<i64> {
        self.group_registry_takes
            .lock()
            .expect("the group registry takes lock")
            .clone()
    }

    fn failure(&self, kind: Failure) -> Option<TagBudgetRowsFailure> {
        match kind {
            Failure::None => None,
            Failure::Query => Some(query_failure()),
            Failure::Decode => Some(decode_failure()),
        }
    }

    fn take(take: i64) -> usize {
        usize::try_from(take).unwrap_or(usize::MAX)
    }
}

#[async_trait]
impl TagRows for RecordingRows {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.tag_queries
            .lock()
            .expect("the tag queries lock")
            .push(names.clone());
        if let Some(failure) = self.failure(self.tag_failure) {
            return Err(failure);
        }
        Ok(self
            .tags
            .iter()
            .filter(|row| names.contains(&row.tag_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(&self, take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        self.registry_takes
            .lock()
            .expect("the registry takes lock")
            .push(take);
        if let Some(failure) = self.failure(self.tag_failure) {
            return Err(failure);
        }
        Ok(self.tags.iter().take(Self::take(take)).cloned().collect())
    }
}

#[async_trait]
impl GroupRows for RecordingRows {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.group_queries
            .lock()
            .expect("the group queries lock")
            .push(names.clone());
        if let Some(failure) = self.failure(self.group_failure) {
            return Err(failure);
        }
        Ok(self
            .groups
            .iter()
            .filter(|row| names.contains(&row.access_group_name))
            .cloned()
            .collect())
    }

    async fn registry_rows(
        &self,
        take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        self.group_registry_takes
            .lock()
            .expect("the group registry takes lock")
            .push(take);
        if let Some(failure) = self.failure(self.group_failure) {
            return Err(failure);
        }
        Ok(self.groups.iter().take(Self::take(take)).cloned().collect())
    }
}
