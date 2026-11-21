use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{
    BudgetResetCalculator, DefaultTeamEnrollment, NoDefaultTeams, PortError,
};
use betterlite_auth::auth_user_lookup::store::{DatabaseUserStore, UserStore};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::team_parent_limits::error::{Rejection, RejectionDetail, TeamLimitError};
use crate::team_request_contracts::create::NewTeamRequest;
use crate::team_request_contracts::update::UpdateTeamRequest;

pub(super) fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(super) fn organization(text: &str) -> OrganizationTable {
    serde_json::from_str(text).unwrap()
}

pub(super) fn team(text: &str) -> LiteLLMTeamTable {
    LiteLLMTeamTable::try_from(data(text)).unwrap()
}

pub(super) fn new_team(text: &str) -> NewTeamRequest {
    NewTeamRequest::from_value(&data(text)).unwrap()
}

pub(super) fn update_team(text: &str) -> UpdateTeamRequest {
    UpdateTeamRequest::from_value(&data(text)).unwrap()
}

pub(super) fn caller(text: &str) -> UserAPIKeyAuth {
    let JsonData::Object(fields) = data(text) else {
        panic!("a caller fixture is an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) fn rejected(result: Result<(), TeamLimitError>) -> Rejection {
    match result {
        Err(TeamLimitError::Rejected(rejection)) => rejection,
        other => panic!("expected a ceiling rejection, got {other:?}"),
    }
}

// the HTTP 400 detail.error text a direct check carries
pub(super) fn error_text(rejection: &Rejection) -> String {
    match rejection.detail() {
        RejectionDetail::Error { error } => error,
        RejectionDetail::Text(text) => panic!("expected detail.error, got the string {text}"),
    }
}

pub(super) fn allocation_text(rejection: &Rejection) -> String {
    match rejection.detail() {
        RejectionDetail::Text(text) => text,
        RejectionDetail::Error { error } => panic!("expected a string detail, got {error}"),
    }
}

// the cache tier get_user_object reads first; reads and writes are counted to prove whether a lookup ran
#[derive(Default)]
pub(super) struct RecordingCache {
    pub(super) user: Option<LiteLLMUserTable>,
    pub(super) reads: Mutex<Vec<String>>,
    pub(super) writes: AtomicUsize,
}

impl RecordingCache {
    pub(super) fn read_keys(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }

    pub(super) fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl UserCache for RecordingCache {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        self.reads.lock().unwrap().push(key.to_owned());
        Ok(self.user.clone())
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct UnusedReset;

impl BudgetResetCalculator for UnusedReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new(
            "a lookup without upsert never computes a reset time",
        ))
    }
}

pub(super) fn lookup(cache: &Arc<RecordingCache>, store: Option<Arc<dyn UserStore>>) -> UserLookup {
    let cache_port: Arc<dyn UserCache> = Arc::<RecordingCache>::clone(cache);
    let enrollment: Arc<dyn DefaultTeamEnrollment> = Arc::new(NoDefaultTeams);
    UserLookup {
        cache: cache_port,
        store,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(UnusedReset),
        enrollment,
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database(seed: &str) -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["team-parent-limits-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    pool.get().await.unwrap().batch_execute(seed).await.unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

pub(super) fn user_store(store: &PostgresStore) -> Arc<DatabaseUserStore> {
    Arc::new(DatabaseUserStore {
        reader: store.clone(),
        writer: store.clone(),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    })
}

pub(super) async fn count(store: &PostgresStore, sql: &str) -> i64 {
    let client = store.session().await.unwrap();
    client.query_one(sql, &[]).await.unwrap().get(0)
}

pub(super) async fn close(fixture: PostgresFixture, store: PostgresStore) {
    if let Some(pool) = store.pool.as_ref() {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}
