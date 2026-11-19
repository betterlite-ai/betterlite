use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::errors::DbError as RepositoryFault;
use betterlite_models::decisions::Json;
use serde::de::DeserializeOwned;

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::payload::{self, PayloadError};
use crate::auth_read::table::AuthReadError;
use crate::auth_resource_grants::rows::RowError;
use crate::auth_tag_budgets::cache::{CachedValue, TagBudgetCache};
use crate::auth_tag_budgets::groups::ModelAccessGroupBudget;
use crate::auth_tag_budgets::rows::{BudgetRows, ModelAccessGroupBudgetRow, TagBudgetRowsFailure};

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
            let wire = format!(r#"{{"tag_name":"tag-{index}","models":[],"spend":0}}"#);
            serde_json::from_str(&wire).expect("the generated tag row")
        })
        .collect()
}

pub(super) fn named_group_rows(count: usize) -> Vec<ModelAccessGroupBudgetRow> {
    (0..count)
        .map(|index| {
            let wire = format!(r#"{{"access_group_name":"group-{index}","spend":0}}"#);
            serde_json::from_str(&wire).expect("the generated group row")
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

// The cached value for one write, encoded as AuthCache::async_set encodes it.
fn stored(value: CachedValue<'_>) -> Json {
    match value {
        CachedValue::Tag(tag) => payload::encode(tag).expect("the tag encodes"),
        CachedValue::Group(group) => payload::encode(group).expect("the group encodes"),
        CachedValue::Registry(answer) => answer.clone(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadKind {
    Tag,
    Group,
    Registry,
}

pub(super) struct RecordingCache {
    values: Mutex<BTreeMap<String, Json>>,
    once: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<(String, f64)>>,
    ttl: f64,
    failing_read: Option<ReadKind>,
    fail_write_after: Option<usize>,
}

impl RecordingCache {
    pub(super) fn new(ttl: f64) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            once: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            ttl,
            failing_read: None,
            fail_write_after: None,
        }
    }

    pub(super) fn with_tag(self, row: &TagTable) -> Self {
        let key = tag_key(&row.tag_name);
        self.cached(key, CachedValue::Tag(row))
    }

    pub(super) fn with_tag_hit_once(self, row: &TagTable) -> Self {
        let key = tag_key(&row.tag_name);
        self.once
            .lock()
            .expect("the scripted values lock")
            .insert(key, stored(CachedValue::Tag(row)));
        self
    }

    pub(super) fn with_group(self, budget: &ModelAccessGroupBudget) -> Self {
        let key = group_key(&budget.access_group_name);
        self.cached(key, CachedValue::Group(budget))
    }

    pub(super) fn with_group_hit_once(self, budget: &ModelAccessGroupBudget) -> Self {
        let key = group_key(&budget.access_group_name);
        self.once
            .lock()
            .expect("the scripted values lock")
            .insert(key, stored(CachedValue::Group(budget)));
        self
    }

    pub(super) fn with_registry(self, key: &str, answer: Json) -> Self {
        self.values
            .lock()
            .expect("the cache values lock")
            .insert(key.to_owned(), answer);
        self
    }

    fn cached(self, key: String, value: CachedValue<'_>) -> Self {
        self.values
            .lock()
            .expect("the cache values lock")
            .insert(key, stored(value));
        self
    }

    pub(super) fn failing_tag_read(mut self) -> Self {
        self.failing_read = Some(ReadKind::Tag);
        self
    }

    pub(super) fn failing_group_read(mut self) -> Self {
        self.failing_read = Some(ReadKind::Group);
        self
    }

    pub(super) fn failing_registry_read(mut self) -> Self {
        self.failing_read = Some(ReadKind::Registry);
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
        let reads = self.reads();
        reads
            .into_iter()
            .filter(|key| key.starts_with(prefix))
            .collect()
    }

    pub(super) fn writes(&self) -> Vec<(String, f64)> {
        self.writes.lock().expect("the cache writes lock").clone()
    }

    pub(super) fn registry_answer(&self, key: &str) -> Option<Json> {
        self.values
            .lock()
            .expect("the cache values lock")
            .get(key)
            .cloned()
    }

    pub(super) fn prefixed_writes(&self, prefix: &str) -> Vec<(String, f64)> {
        let writes = self.writes();
        writes
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

    fn scripted(&self, key: &str) -> Option<Json> {
        self.once
            .lock()
            .expect("the scripted values lock")
            .remove(key)
    }

    fn cached_value(&self, key: &str) -> Option<Json> {
        self.values
            .lock()
            .expect("the cache values lock")
            .get(key)
            .cloned()
    }

    fn probe(&self, key: &str, fail: bool) -> Result<Option<Json>, PayloadError> {
        self.record_read(key);
        if fail {
            return Err(payload_error());
        }
        if let Some(scripted) = self.scripted(key) {
            return Ok(Some(scripted));
        }
        Ok(self.cached_value(key))
    }

    fn write_count(&self) -> usize {
        self.writes.lock().expect("the cache writes lock").len()
    }

    fn push_write(&self, key: &str, ttl: f64) {
        self.writes
            .lock()
            .expect("the cache writes lock")
            .push((key.to_owned(), ttl));
    }

    fn record_write(&self, key: &str, ttl: f64) -> Result<(), AuthCacheError> {
        if self
            .fail_write_after
            .is_some_and(|allowed| self.write_count() >= allowed)
        {
            return Err(AuthCacheError::Payload(Box::new(payload_error())));
        }
        self.push_write(key, ttl);
        Ok(())
    }
}

// a stored payload the model cannot read is a miss, the way AuthCache's typed read treats it
fn decoded<Value>(value: Option<Json>) -> Result<Option<Value>, PayloadError>
where
    Value: DeserializeOwned,
{
    value.map_or(Ok(None), |present| payload::decode(&present))
}

#[async_trait]
impl TagBudgetCache for RecordingCache {
    async fn read_tag(&self, key: &str) -> Result<Option<TagTable>, PayloadError> {
        decoded(self.probe(key, self.failing_read == Some(ReadKind::Tag))?)
    }

    async fn read_group(&self, key: &str) -> Result<Option<ModelAccessGroupBudget>, PayloadError> {
        decoded(self.probe(key, self.failing_read == Some(ReadKind::Group))?)
    }

    async fn read_registry(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        let found = self
            .probe(key, self.failing_read == Some(ReadKind::Registry))
            .map_err(|error| AuthCacheError::Payload(Box::new(error)))?;
        Ok(found)
    }

    async fn write(
        &self,
        key: &str,
        value: CachedValue<'_>,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        self.record_write(key, ttl)?;
        self.values
            .lock()
            .expect("the cache values lock")
            .insert(key.to_owned(), stored(value));
        Ok(())
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

fn matching<Row>(rows: &[Row], names: &[String], name: fn(&Row) -> &str) -> Vec<Row>
where
    Row: Clone,
{
    rows.iter()
        .filter(|row| names.iter().any(|wanted| wanted.as_str() == name(row)))
        .cloned()
        .collect()
}

fn limited<Row>(rows: &[Row], take: i64) -> Vec<Row>
where
    Row: Clone,
{
    rows.iter().take(take_of(take)).cloned().collect()
}

fn take_of(take: i64) -> usize {
    usize::try_from(take).unwrap_or(usize::MAX)
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

    fn failure(kind: Failure) -> Option<TagBudgetRowsFailure> {
        match kind {
            Failure::None => None,
            Failure::Query => Some(query_failure()),
            Failure::Decode => Some(decode_failure()),
        }
    }

    fn record(log: &Mutex<Vec<Vec<String>>>, names: Vec<String>) {
        log.lock().expect("the queries lock").push(names);
    }

    fn record_take(log: &Mutex<Vec<i64>>, take: i64) {
        log.lock().expect("the takes lock").push(take);
    }
}

#[async_trait]
impl BudgetRows<TagTable> for RecordingRows {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Self::record(&self.tag_queries, names.clone());
        if let Some(failure) = Self::failure(self.tag_failure) {
            return Err(failure);
        }
        Ok(matching(&self.tags, &names, |row| &row.tag_name))
    }

    async fn registry_rows(&self, take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure> {
        Self::record_take(&self.registry_takes, take);
        if let Some(failure) = Self::failure(self.tag_failure) {
            return Err(failure);
        }
        Ok(limited(&self.tags, take))
    }
}

#[async_trait]
impl BudgetRows<ModelAccessGroupBudgetRow> for RecordingRows {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Self::record(&self.group_queries, names.clone());
        if let Some(failure) = Self::failure(self.group_failure) {
            return Err(failure);
        }
        Ok(matching(&self.groups, &names, |row| &row.access_group_name))
    }

    async fn registry_rows(
        &self,
        take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure> {
        Self::record_take(&self.group_registry_takes, take);
        if let Some(failure) = Self::failure(self.group_failure) {
            return Err(failure);
        }
        Ok(limited(&self.groups, take))
    }
}
