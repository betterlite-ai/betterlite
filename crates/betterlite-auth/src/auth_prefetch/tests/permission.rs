#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::database_failures::deadline::{DbLookupDeadlineExceeded, DbLookupError};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_faults::{DbError as FaultDbError, ErrorType};
use betterlite_db::postgres_store::errors::DbError;
use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;

use super::fixtures;
use crate::auth_cache::AuthCacheError;
use crate::auth_cache::payload::PayloadError;
use crate::auth_prefetch::permission::lookup::{
    MISSING_PERMISSION_MESSAGE, MISSING_PERMISSION_STATUS, ObjectPermissionCache,
    ObjectPermissionSource, PermissionCacheError, PermissionError, PermissionStores,
    get_object_permission,
};
use crate::auth_prefetch::permission::store::{BoundedPermissionStore, ObjectPermissionStore};
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, ReadContext, Update,
};

use super::support::DropFlag;

const MANAGEMENT_TTL: f64 = 3600.0;
const LOOKUP_LABEL: &str = "object_permission";

fn permission_model(permission_id: &str) -> LiteLLMObjectPermissionTable {
    let row = fixtures::permission_row(permission_id);
    let wire = serde_json::to_value(row).expect("the row serializes");
    serde_json::from_value(wire).expect("the permission model validates")
}

fn cache_failure() -> PermissionCacheError {
    let payload = serde_json::from_str::<String>("{").expect_err("the fixture is not json");
    PermissionCacheError::from(AuthCacheError::from(PayloadError::Json(payload)))
}

fn deadline_error() -> DbLookupError {
    DbLookupError::Deadline(DbLookupDeadlineExceeded {
        lookup: LOOKUP_LABEL.to_owned(),
        deadline_seconds: 0.0,
        cause: None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreFailure {
    Record,
    NoDatabase,
    Lookup,
}

struct FakePermissionStore {
    row: Mutex<Option<JsonData>>,
    failure: Mutex<Option<StoreFailure>>,
    calls: Mutex<Vec<String>>,
}

impl FakePermissionStore {
    fn returning(row: JsonData) -> Self {
        Self {
            row: Mutex::new(Some(row)),
            ..Self::empty()
        }
    }

    fn missing() -> Self {
        Self::empty()
    }

    fn failing(failure: StoreFailure) -> Self {
        Self {
            failure: Mutex::new(Some(failure)),
            ..Self::empty()
        }
    }

    fn empty() -> Self {
        Self {
            row: Mutex::new(None),
            failure: Mutex::new(None),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ObjectPermissionStore for FakePermissionStore {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id.to_owned());
        let failure = *self.failure.lock().unwrap_or_else(PoisonError::into_inner);
        match failure {
            Some(StoreFailure::Record) => {
                Err(PermissionError::Repository(DbError::InvalidRecord {
                    source: TableError::MissingRow,
                }))
            }
            Some(StoreFailure::NoDatabase) => {
                Err(PermissionError::Repository(DbError::NoDbConnected))
            }
            Some(StoreFailure::Lookup) => Err(PermissionError::Lookup(deadline_error())),
            None => Ok(self
                .row
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CacheSet {
    key: String,
    permission: LiteLLMObjectPermissionTable,
    ttl: f64,
}

struct FakePermissionCache {
    permission: Mutex<Option<LiteLLMObjectPermissionTable>>,
    fail_read: AtomicBool,
    fail_write: AtomicBool,
    gets: Mutex<Vec<String>>,
    sets: Mutex<Vec<CacheSet>>,
    management_ttl: f64,
}

impl FakePermissionCache {
    fn empty() -> Self {
        Self {
            permission: Mutex::new(None),
            fail_read: AtomicBool::new(false),
            fail_write: AtomicBool::new(false),
            gets: Mutex::new(Vec::new()),
            sets: Mutex::new(Vec::new()),
            management_ttl: MANAGEMENT_TTL,
        }
    }

    fn returning(permission: LiteLLMObjectPermissionTable) -> Self {
        let cache = Self::empty();
        *cache
            .permission
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(permission);
        cache
    }

    fn gets(&self) -> Vec<String> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn sets(&self) -> Vec<CacheSet> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ObjectPermissionCache for FakePermissionCache {
    async fn get(
        &self,
        key: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionCacheError> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        if self.fail_read.load(Ordering::SeqCst) {
            return Err(cache_failure());
        }
        Ok(self
            .permission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn set(
        &self,
        key: &str,
        permission: &LiteLLMObjectPermissionTable,
        ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheSet {
                key: key.to_owned(),
                permission: permission.clone(),
                ttl,
            });
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(cache_failure());
        }
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        self.management_ttl
    }
}

struct FakeTable {
    pending: bool,
    row: Option<JsonData>,
    filters: Mutex<Vec<String>>,
    dropped: Arc<AtomicBool>,
}

impl FakeTable {
    fn hanging(dropped: Arc<AtomicBool>) -> Self {
        Self {
            pending: true,
            row: None,
            filters: Mutex::new(Vec::new()),
            dropped,
        }
    }
}

#[async_trait]
impl AuthTable for FakeTable {
    type Row = JsonData;
    type Key = String;
    type Filter = String;
    type Include = ();
    type Data = ();
    type Failure = TableError;

    async fn find_unique(
        &self,
        request: FindOne<String, ()>,
    ) -> Result<Option<JsonData>, AuthReadError<TableError>> {
        self.filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.filter);
        if self.pending {
            let _held = DropFlag(Arc::clone(&self.dropped));
            return std::future::pending().await;
        }
        Ok(self.row.clone())
    }

    async fn find_first(
        &self,
        _request: FindOne<String, ()>,
    ) -> Result<Option<JsonData>, AuthReadError<TableError>> {
        Err(unread())
    }

    async fn find_many(
        &self,
        _request: FindMany<String, ()>,
    ) -> Result<Vec<JsonData>, AuthReadError<TableError>> {
        Err(unread())
    }

    async fn update(
        &self,
        _request: Update<String, ()>,
    ) -> Result<Option<JsonData>, AuthReadError<TableError>> {
        Err(unread())
    }

    async fn create(
        &self,
        _request: Create<(), ()>,
    ) -> Result<JsonData, AuthReadError<TableError>> {
        Err(unread())
    }
}

fn unread() -> AuthReadError<TableError> {
    AuthReadError::Repository(DbError::InvalidRecord {
        source: TableError::MissingRow,
    })
}

fn bounded(table: Arc<FakeTable>, deadline_seconds: f64) -> BoundedPermissionStore<FakeTable> {
    BoundedPermissionStore {
        table,
        context: ReadContext {
            default_deadline_seconds: deadline_seconds,
            tracker: Arc::new(DbLookupStallTracker {
                clock: SystemClock,
                last_hit: Mutex::new(None),
            }),
            tasks: TaskTracker::new(),
        },
    }
}

#[tokio::test]
async fn no_db_even_with_cache_hit() {
    let cache = FakePermissionCache::returning(permission_model("perm-1"));
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: None,
    };
    let outcome = get_object_permission(&source, "perm-1", false).await;
    assert!(matches!(outcome, Err(PermissionError::NoDatabase)));
    assert!(cache.gets().is_empty());
    assert_eq!(
        PermissionError::NoDatabase.to_string(),
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
}

#[tokio::test]
async fn normal_permission_cache_hit() {
    let cache = FakePermissionCache::returning(permission_model("perm-1"));
    let normal = FakePermissionStore::missing();
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let permission = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the cache hit")
        .expect("the cached permission");
    assert_eq!(permission.object_permission_id, "perm-1");
    assert!(normal.calls().is_empty());
    assert!(writer.calls().is_empty());
    assert!(cache.sets().is_empty());
}

#[tokio::test]
async fn db_only_skips_cache_and_uses_writer() {
    let cache = FakePermissionCache::returning(permission_model("perm-1"));
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-normal"));
    let writer = FakePermissionStore::returning(fixtures::permission_row("perm-writer"));
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let permission = get_object_permission(&source, "perm-requested", true)
        .await
        .expect("the database-only lookup")
        .expect("the stored permission");
    assert_eq!(permission.object_permission_id, "perm-writer");
    assert!(cache.gets().is_empty());
    assert!(normal.calls().is_empty());
    assert_eq!(writer.calls(), vec!["perm-requested".to_owned()]);
    assert_eq!(
        cache.sets(),
        vec![CacheSet {
            key: "object_permission_id:perm-requested".to_owned(),
            permission,
            ttl: MANAGEMENT_TTL,
        }]
    );
}

#[tokio::test]
async fn permission_cache_read_error_propagates() {
    let cache = FakePermissionCache::empty();
    cache.fail_read.store(true, Ordering::SeqCst);
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", false).await;
    assert!(matches!(outcome, Err(PermissionError::Cache(_))));
    assert!(normal.calls().is_empty());

    let permission = get_object_permission(&source, "perm-1", true)
        .await
        .expect("the database-only lookup skips the cache")
        .expect("the stored permission");
    assert_eq!(permission.object_permission_id, "perm-1");
}

#[tokio::test]
async fn permission_lookup_label_and_filter() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    get_object_permission(&source, "perm-1", false)
        .await
        .expect("the stored permission");
    assert_eq!(normal.calls(), vec!["perm-1".to_owned()]);

    // a reached table records the id it was asked for
    let table = Arc::new(FakeTable {
        pending: false,
        row: None,
        filters: Mutex::new(Vec::new()),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let store = bounded(Arc::clone(&table), 60.0);
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &store,
            writer: &store,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", true).await;
    assert!(matches!(
        outcome,
        Err(PermissionError::MissingInDatabaseOnly)
    ));
    let filters = table
        .filters
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(filters, vec!["perm-1".to_owned()]);

    // the shared deadline names the lookup after its table
    let hanging = Arc::new(FakeTable::hanging(Arc::new(AtomicBool::new(false))));
    let store = bounded(hanging, 0.0);
    let outcome = store.find_permission("perm-1").await;
    let Err(PermissionError::Lookup(DbLookupError::Deadline(error))) = outcome else {
        panic!("the deadline crosses the boundary");
    };
    assert_eq!(error.lookup, LOOKUP_LABEL);
}

#[tokio::test]
async fn missing_permission_normal_none() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::missing();
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the missing row is not an error");
    assert!(outcome.is_none());
    assert!(cache.sets().is_empty());
}

#[tokio::test]
async fn missing_permission_db_only_403() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::missing();
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", true).await;
    let Err(error) = outcome else {
        panic!("the database-only miss raises");
    };
    assert_eq!(error.to_string(), MISSING_PERMISSION_MESSAGE);
    assert!(matches!(error, PermissionError::MissingInDatabaseOnly));
    assert_eq!(MISSING_PERMISSION_STATUS, 403);
    assert!(cache.sets().is_empty());
}

#[tokio::test]
async fn permission_row_validated_then_cached() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let permission = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the stored permission")
        .expect("the row is present");
    assert_eq!(permission, permission_model("perm-1"));
    assert_eq!(
        cache.sets(),
        vec![CacheSet {
            key: "object_permission_id:perm-1".to_owned(),
            permission,
            ttl: MANAGEMENT_TTL,
        }]
    );
}

#[tokio::test]
async fn normal_db_validation_write_error_none() {
    let cache = FakePermissionCache::empty();
    let invalid = FakePermissionStore::returning(JsonData::String("not a row".to_owned()));
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &invalid,
            writer: &writer,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the validation failure is swallowed");
    assert!(outcome.is_none());
    assert!(cache.sets().is_empty());

    let cache = FakePermissionCache::empty();
    cache.fail_write.store(true, Ordering::SeqCst);
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the cache write failure is swallowed");
    assert!(outcome.is_none());
    assert_eq!(cache.sets().len(), 1);
}

#[tokio::test]
async fn db_only_error_propagates() {
    let cache = FakePermissionCache::empty();
    let failed = FakePermissionStore::failing(StoreFailure::Record);
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &failed,
            writer: &failed,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", true).await;
    assert!(matches!(outcome, Err(PermissionError::Repository(_))));

    let invalid = FakePermissionStore::returning(JsonData::String("not a row".to_owned()));
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &invalid,
            writer: &invalid,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", true).await;
    assert!(matches!(outcome, Err(PermissionError::Validation(_))));

    let cache = FakePermissionCache::empty();
    cache.fail_write.store(true, Ordering::SeqCst);
    let stored = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &stored,
            writer: &stored,
        }),
    };
    let outcome = get_object_permission(&source, "perm-1", true).await;
    assert!(matches!(outcome, Err(PermissionError::Cache(_))));
}

#[tokio::test]
async fn permission_deadline_uses_shared_boundary() {
    let dropped = Arc::new(AtomicBool::new(false));
    let table = Arc::new(FakeTable::hanging(Arc::clone(&dropped)));
    let store = bounded(table, 0.0);
    let outcome = store.find_permission("perm-1").await;
    let Err(PermissionError::Lookup(DbLookupError::Deadline(error))) = outcome else {
        panic!("the bounded lookup reports the shared deadline");
    };
    assert_eq!(error.lookup, LOOKUP_LABEL);
    assert_eq!(error.deadline_seconds, 0.0);
}

#[tokio::test]
async fn permission_no_negative_cache() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::missing();
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let first = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the first miss is not an error");
    let second = get_object_permission(&source, "perm-1", false)
        .await
        .expect("the second miss is not an error");
    assert!(first.is_none());
    assert!(second.is_none());
    assert_eq!(
        normal.calls(),
        vec!["perm-1".to_owned(), "perm-1".to_owned()]
    );
    assert!(cache.sets().is_empty());
}

#[tokio::test]
async fn permission_metrics_record_actual_outcome() {
    let cache = FakePermissionCache::empty();
    let normal = FakePermissionStore::returning(fixtures::permission_row("perm-1"));
    let writer = FakePermissionStore::missing();
    let source = ObjectPermissionSource {
        cache: &cache,
        stores: Some(PermissionStores {
            normal: &normal,
            writer: &writer,
        }),
    };
    let success = get_object_permission(&source, "perm-1", false).await;
    assert!(success.is_ok());

    assert!(PermissionError::NoDatabase.database_fault().is_none());
    assert!(
        PermissionError::MissingInDatabaseOnly
            .database_fault()
            .is_none()
    );
    assert!(
        PermissionError::Cache(cache_failure())
            .database_fault()
            .is_none()
    );
    assert!(
        PermissionError::Lookup(deadline_error())
            .database_fault()
            .is_none()
    );
    let validation = serde_json::from_str::<String>("{").expect_err("the fixture is not json");
    assert!(
        PermissionError::Validation(validation)
            .database_fault()
            .is_none()
    );

    let lookup = PermissionError::Lookup(DbLookupError::Database(FaultDbError {
        error_type: ErrorType::Connect,
        message: "connect failed".to_owned(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    }));
    let fault = lookup
        .database_fault()
        .expect("the lookup fault is a db fault");
    assert_eq!(fault.error_type, ErrorType::Connect);

    let repository = PermissionError::Repository(DbError::InvalidRecord {
        source: TableError::MissingRow,
    });
    let fault = repository
        .database_fault()
        .expect("the repository fault is a db fault");
    assert_eq!(fault.error_type, ErrorType::Prisma);
    assert_eq!(
        PermissionError::Repository(DbError::NoDbConnected)
            .database_fault()
            .map(|fault| fault.error_type),
        None
    );
}

#[tokio::test]
async fn permission_cancel_propagates() {
    let dropped = Arc::new(AtomicBool::new(false));
    let table = Arc::new(FakeTable::hanging(Arc::clone(&dropped)));
    let store = bounded(table, 60.0);
    let cancelled = timeout(Duration::from_millis(50), store.find_permission("perm-1")).await;
    assert!(cancelled.is_err());
    timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the bounded lookup was dropped");
}
