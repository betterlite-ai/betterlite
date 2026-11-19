use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_read::table::AuthReadError;
use crate::auth_resource_grants::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreCache, ManagedVectorStoreRows, VectorStoreRow,
    VectorStoreRowsFailure,
};
use crate::auth_resource_grants::search::{SearchToolFailure, SearchToolGrant};
use crate::auth_resource_grants::vector_store::{
    ObjectPermissionRows, VectorStoreIds, VectorStoreLookupFailure,
};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) fn user_key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(fields).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

pub(super) fn team(fields: &str) -> LiteLLMTeamTable {
    serde_json::from_str(fields).expect("the team fixture decodes")
}

pub(super) fn permission(row: &str) -> LiteLLMObjectPermissionTable {
    serde_json::from_str(row).expect("the permission row decodes")
}

pub(super) fn body(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).expect("the request body decodes")
}

pub(super) fn row(text: &str) -> VectorStoreRow {
    serde_json::from_str(text).expect("the raw managed row decodes")
}

pub(super) fn managed_store(text: &str) -> ManagedVectorStoresTable {
    serde_json::from_str(text).expect("the managed vector store row decodes")
}

pub(super) fn proxy_exception(message: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: message.to_owned(),
        error_type: "auth_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(401),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

pub(super) fn payload_error() -> PayloadError {
    PayloadError::Json(serde_json::from_str::<i64>("not a number").expect_err("the invalid json"))
}

fn permission_rows(rows: &[(&str, &str)]) -> BTreeMap<String, LiteLLMObjectPermissionTable> {
    rows.iter()
        .map(|(id, row)| ((*id).to_owned(), permission(row)))
        .collect()
}

// the fault a repository answers with when the pool hands out no connection
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

fn lookup_failure() -> VectorStoreLookupFailure {
    VectorStoreLookupFailure {
        source: Box::new(AuthReadError::Lookup(
            DbLookupError::Database(no_database()),
        )),
    }
}

pub(super) struct RecordingPermissions {
    rows: BTreeMap<String, LiteLLMObjectPermissionTable>,
    calls: Mutex<Vec<String>>,
    failing: bool,
}

impl RecordingPermissions {
    pub(super) fn new(rows: &[(&str, &str)]) -> Self {
        Self {
            rows: permission_rows(rows),
            calls: Mutex::new(Vec::new()),
            failing: false,
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            rows: BTreeMap::new(),
            calls: Mutex::new(Vec::new()),
            failing: true,
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the permission calls lock")
            .clone()
    }
}

#[async_trait]
impl ObjectPermissionRows for RecordingPermissions {
    async fn by_id(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, VectorStoreLookupFailure> {
        self.calls
            .lock()
            .expect("the permission calls lock")
            .push(object_permission_id.to_owned());
        if self.failing {
            return Err(lookup_failure());
        }
        Ok(self.rows.get(object_permission_id).cloned())
    }
}

pub(super) struct RecordingSelection {
    ids: Vec<String>,
    calls: AtomicUsize,
}

impl RecordingSelection {
    pub(super) fn new(ids: &[&str]) -> Self {
        Self {
            ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl VectorStoreIds for RecordingSelection {
    fn vector_store_ids(&self, _body: &BTreeMap<String, JsonData>) -> Vec<String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.ids.clone()
    }
}

pub(super) struct RecordingCache {
    hits: BTreeMap<String, ManagedVectorStoresTable>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<(String, f64)>>,
    ttl: f64,
    fail_read: bool,
    fail_write_after: Option<usize>,
}

impl RecordingCache {
    pub(super) fn new(ttl: f64) -> Self {
        Self {
            hits: BTreeMap::new(),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            ttl,
            fail_read: false,
            fail_write_after: None,
        }
    }

    pub(super) fn with_hit(mut self, uuid: &str, stored: ManagedVectorStoresTable) -> Self {
        self.hits
            .insert(format!("managed_vector_store_id:{uuid}"), stored);
        self
    }

    pub(super) fn failing_read(mut self) -> Self {
        self.fail_read = true;
        self
    }

    pub(super) fn failing_write_after(mut self, written: usize) -> Self {
        self.fail_write_after = Some(written);
        self
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the cache reads lock").clone()
    }

    pub(super) fn writes(&self) -> Vec<(String, f64)> {
        self.writes.lock().expect("the cache writes lock").clone()
    }
}

#[async_trait]
impl ManagedVectorStoreCache for RecordingCache {
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError> {
        self.reads
            .lock()
            .expect("the cache reads lock")
            .push(key.to_owned());
        if self.fail_read {
            return Err(payload_error());
        }
        Ok(self.hits.get(key).cloned())
    }

    fn management_ttl(&self) -> f64 {
        self.ttl
    }

    async fn write(
        &self,
        key: &str,
        _row: &ManagedVectorStoresTable,
        ttl: f64,
    ) -> Result<(), AuthCacheError> {
        let over_budget = self.fail_write_after.is_some_and(|allowed| {
            self.writes.lock().expect("the cache writes lock").len() >= allowed
        });
        if over_budget {
            return Err(AuthCacheError::Payload(Box::new(payload_error())));
        }
        self.writes
            .lock()
            .expect("the cache writes lock")
            .push((key.to_owned(), ttl));
        Ok(())
    }
}

fn rows_failure() -> VectorStoreRowsFailure {
    VectorStoreRowsFailure {
        source: Box::new(AuthReadError::Lookup(
            DbLookupError::Database(no_database()),
        )),
    }
}

pub(super) struct RecordingRows {
    rows: Vec<VectorStoreRow>,
    queries: Mutex<Vec<(Vec<String>, i64)>>,
    failing: bool,
}

impl RecordingRows {
    pub(super) fn new(rows: Vec<VectorStoreRow>) -> Self {
        Self {
            rows,
            queries: Mutex::new(Vec::new()),
            failing: false,
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            rows: Vec::new(),
            queries: Mutex::new(Vec::new()),
            failing: true,
        }
    }

    pub(super) fn queries(&self) -> Vec<(Vec<String>, i64)> {
        self.queries.lock().expect("the row queries lock").clone()
    }
}

#[async_trait]
impl ManagedVectorStoreRows for RecordingRows {
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure> {
        self.queries
            .lock()
            .expect("the row queries lock")
            .push((query.uuids, query.take));
        if self.failing {
            return Err(rows_failure());
        }
        Ok(self.rows.clone())
    }
}

#[derive(Clone, Copy)]
pub(super) enum GrantAnswer {
    Allow,
    Deny,
    Fail,
}

pub(super) struct ScriptedGrant {
    answer: GrantAnswer,
    calls: Mutex<Vec<String>>,
}

impl ScriptedGrant {
    pub(super) fn new(answer: GrantAnswer) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the grant calls lock").clone()
    }
}

impl SearchToolGrant for ScriptedGrant {
    fn authorize(&self, name: &str) -> Result<(), SearchToolFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        match self.answer {
            GrantAnswer::Allow => Ok(()),
            GrantAnswer::Deny => Err(SearchToolFailure::Denied(Box::new(proxy_exception(
                "the scripted denial",
            )))),
            GrantAnswer::Fail => Err(SearchToolFailure::Lookup {
                message: "the scripted lookup failure".to_owned(),
            }),
        }
    }
}
