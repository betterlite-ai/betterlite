use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::OrgCache;
use betterlite_auth::auth_org_lookup::store::DatabaseOrgStore;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_user_lookup::store::DatabaseUserStore;
use betterlite_config::sections::key_limit_policy::UpperboundKeyGenerateParams;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use jiff::tz::TimeZoneDatabase;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::key_fields::decode::TypedValue;
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::update::UpdateKeyRequest;
use crate::key_limit_policy::error::{KeyLimitError, Rejection};
use crate::key_limit_policy::reads::{
    KeyLimitReadError, KeyLimitReads, KeyOwner, PostgresKeyLimitReads,
};
use crate::key_limit_policy::upperbound::TimeContext;
use crate::team_parent_limits::error::RejectionDetail;

pub(super) fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> BTreeMap<String, JsonData> {
    match data(text) {
        JsonData::Object(fields) => fields,
        other @ (JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_)) => panic!("expected an object, got {other:?}"),
    }
}

pub(super) fn generate(text: &str) -> GenerateKeyRequest {
    GenerateKeyRequest::read(&TypedValue { json: data(text) }).unwrap()
}

pub(super) fn update(text: &str) -> UpdateKeyRequest {
    UpdateKeyRequest::read(&TypedValue { json: data(text) }).unwrap()
}

pub(super) fn team(text: &str) -> LiteLLMTeamTableCachedObj {
    LiteLLMTeamTableCachedObj::try_from(data(text)).unwrap()
}

pub(super) fn organization(text: &str) -> OrganizationTable {
    serde_json::from_str(text).unwrap()
}

pub(super) fn project(text: &str) -> CachedProject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn keys(text: &str) -> Vec<LiteLLMVerificationToken> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn params(text: &str) -> UpperboundKeyGenerateParams {
    serde_json::from_str(text).unwrap()
}

pub(super) fn rejected<Ok>(result: Result<Ok, KeyLimitError>) -> Rejection
where
    Ok: Debug,
{
    match result {
        Err(KeyLimitError::Rejected(rejection)) => rejection,
        other => panic!("expected a ceiling rejection, got {other:?}"),
    }
}

pub(super) fn error_text(rejection: &Rejection) -> String {
    match rejection.detail() {
        RejectionDetail::Error { error } => error,
        RejectionDetail::Text(text) => panic!("expected detail.error, got the string {text}"),
    }
}

pub(super) fn text_detail(rejection: &Rejection) -> String {
    match rejection.detail() {
        RejectionDetail::Text(text) => text,
        RejectionDetail::Error { error } => panic!("expected a string detail, got {error}"),
    }
}

pub(super) fn time(database: &TimeZoneDatabase) -> TimeContext<'_> {
    let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    TimeContext {
        now,
        timezone: None,
        database,
    }
}

#[derive(Default)]
pub(super) struct FakeReads {
    pub(super) keys: Vec<LiteLLMVerificationToken>,
    pub(super) project: Option<CachedProject>,
    pub(super) memberships: BTreeSet<String>,
    pub(super) queries: Mutex<Vec<(&'static str, String)>>,
    pub(super) user_lookups: Mutex<Vec<String>>,
}

impl FakeReads {
    pub(super) fn with_keys(mut self, keys: Vec<LiteLLMVerificationToken>) -> Self {
        self.keys = keys;
        self
    }

    pub(super) fn with_project(mut self, project: CachedProject) -> Self {
        self.project = Some(project);
        self
    }
}

#[async_trait]
impl KeyLimitReads for FakeReads {
    async fn keys(
        &self,
        owner: KeyOwner<'_>,
    ) -> Result<Vec<LiteLLMVerificationToken>, KeyLimitReadError> {
        self.queries
            .lock()
            .unwrap()
            .push((owner.column().name(), owner.id().to_owned()));
        Ok(self.keys.clone())
    }

    async fn project(&self, _project_id: &str) -> Result<Option<CachedProject>, KeyLimitReadError> {
        Ok(self.project.clone())
    }

    async fn member_organizations(
        &self,
        user_id: &str,
    ) -> Result<BTreeSet<String>, KeyLimitReadError> {
        self.user_lookups.lock().unwrap().push(user_id.to_owned());
        Ok(self.memberships.clone())
    }
}

// the cache trait's fallible answer, so each NoProjects signature stays one line
type CacheAnswer<T> = Result<T, AuthCacheError>;

struct NoProjects;

#[async_trait]
impl OrgCache for NoProjects {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> CacheAnswer<Option<OrganizationTable>> {
        Ok(None)
    }

    async fn write_org(&self, _key: &str, _org: &OrganizationTable, _ttl: f64) -> CacheAnswer<()> {
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> CacheAnswer<Option<CachedProject>> {
        Ok(None)
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> CacheAnswer<()> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

pub(super) fn reads(store: &PostgresStore) -> PostgresKeyLimitReads {
    PostgresKeyLimitReads {
        store: store.clone(),
        projects: OrgLookup {
            store: Some(Arc::new(DatabaseOrgStore {
                reader: store.clone(),
                writer: store.clone(),
                availability: Arc::new(Availability::default()),
                default_deadline_seconds: 5.0_f64,
                tracker: Arc::new(DbLookupStallTracker {
                    clock: SystemClock,
                    last_hit: Mutex::new(None),
                }),
                tasks: TaskTracker::new(),
            })),
            cache: Arc::new(NoProjects),
            in_memory_ttl: 60.0_f64,
        },
        users: user_store(store),
        now: 1.0_f64,
    }
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database(seed: &str) -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["key-limit-policy-tests"])
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

fn user_store(store: &PostgresStore) -> Arc<DatabaseUserStore> {
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

pub(super) async fn close(fixture: PostgresFixture, store: PostgresStore) {
    if let Some(pool) = store.pool.as_ref() {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}
