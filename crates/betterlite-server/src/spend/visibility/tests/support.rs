use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};
use tokio_util::task::TaskTracker;

use crate::spend::visibility::error::VisibilityError;
use crate::spend::visibility::owners::OwnerPair;
use crate::spend::visibility::postgres::PostgresVisibility;
use crate::spend::visibility::query::PayloadQuery;
use crate::spend::visibility::service::SpendLogVisibility;
use crate::spend::visibility::store::{PayloadRow, VisibilityStore};
use crate::team_access::caller::TeamCaller;

// team-a grants alice /spend/logs, team-b grants another route, alice administers team-c, and team "" names her admin
const STORAGE: &str = r#"CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT NOT NULL PRIMARY KEY,
    litellm_call_id TEXT,
    "user" TEXT,
    team_id TEXT,
    messages JSONB,
    response JSONB,
    proxy_server_request JSONB,
    metadata JSONB
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT NOT NULL PRIMARY KEY,
    members_with_roles JSONB NOT NULL DEFAULT '[]',
    team_member_permissions TEXT[]
);
INSERT INTO "LiteLLM_TeamTable" (team_id, members_with_roles, team_member_permissions) VALUES
    ('team-a', '[{"user_id":"alice","role":"user"}]', ARRAY['/spend/logs']),
    ('team-b', '[{"user_id":"alice","role":"user"}]', ARRAY['/key/info']),
    ('team-c', '[{"user_id":"alice","role":"admin"}]', NULL),
    ('team-x', '[{"user_id":"mallory","role":"admin"}]', NULL),
    ('', '[{"user_id":"alice","role":"admin"}]', NULL);
INSERT INTO "LiteLLM_SpendLogs" VALUES
    ('req-own', 'call-shared', 'alice', NULL, '[{"role":"user"}]', '{"id":"resp-1"}', '{"model":"m"}', '{"user_api_key_user_id":"alice"}'),
    ('req-foreign-1', 'call-shared', 'mallory', 'team-x', NULL, NULL, NULL, NULL),
    ('req-foreign-2', 'call-shared', 'mallory', 'team-x', NULL, NULL, NULL, NULL),
    ('req-team', 'call-team', 'bob', 'team-a', NULL, NULL, NULL, NULL),
    ('req-admin-team', 'call-admin-team', 'carol', 'team-c', NULL, NULL, NULL, NULL),
    ('req-no-permission', 'call-no-permission', 'dave', 'team-b', NULL, NULL, NULL, NULL),
    ('req-foreign', 'call-foreign', 'mallory', 'team-x', NULL, NULL, NULL, NULL),
    ('req-blank-team', NULL, 'mallory', '', NULL, NULL, NULL, NULL),
    ('req-anonymous', NULL, NULL, NULL, NULL, NULL, NULL, NULL);"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) struct Seeded {
    pub(super) fixture: PostgresFixture,
    pub(super) pool: Pool,
    pub(super) store: PostgresStore,
}

pub(super) async fn seeded() -> Seeded {
    let url = Environment::parse_from(["spend-visibility-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![STORAGE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .unwrap();
    let store = PostgresStore {
        pool: Some(pool.clone()),
    };
    Seeded {
        fixture,
        pool,
        store,
    }
}

pub(super) async fn finish(seeded: Seeded) {
    seeded.pool.close();
    seeded.fixture.close_async().await.unwrap();
}

pub(super) async fn execute(pool: &Pool, statement: &str) {
    pool.get()
        .await
        .unwrap()
        .batch_execute(statement)
        .await
        .unwrap();
}

pub(super) fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> TeamCaller {
    TeamCaller {
        user_id: user_id.map(str::to_owned),
        user_role,
        team_id: None,
        team_service_account: false,
    }
}

pub(super) fn alice() -> TeamCaller {
    caller(Some("alice"), Some(UserRole::InternalUser))
}

pub(super) fn admin() -> TeamCaller {
    caller(Some("root"), Some(UserRole::ProxyAdmin))
}

pub(super) fn user(user_id: &str, teams: &[&str]) -> LiteLLMUserTable {
    let record = JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "teams".to_owned(),
            JsonData::Array(
                teams
                    .iter()
                    .map(|team| JsonData::String((*team).to_owned()))
                    .collect(),
            ),
        ),
    ]));
    user_record_to_model(Some(&record)).unwrap().unwrap()
}

pub(super) fn visibility(
    store: Arc<dyn VisibilityStore>,
    users: Vec<LiteLLMUserTable>,
) -> SpendLogVisibility {
    let lookup = UserLookup {
        cache: Arc::new(CachedUsers { users }),
        store: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
    };
    SpendLogVisibility {
        store,
        users: Arc::new(lookup),
    }
}

pub(super) fn postgres(seeded: &Seeded, users: Vec<LiteLLMUserTable>) -> SpendLogVisibility {
    let store = PostgresVisibility {
        store: seeded.store.clone(),
    };
    visibility(Arc::new(store), users)
}

pub(super) fn forbidden(outcome: Result<(), VisibilityError>) -> String {
    let Err(VisibilityError::Forbidden { request_id }) = outcome else {
        panic!("expected the spend log 403");
    };
    request_id
}

// the users a cache hit returns; a miss has no database behind it, so the lookup fails
struct CachedUsers {
    users: Vec<LiteLLMUserTable>,
}

#[async_trait]
impl UserCache for CachedUsers {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self
            .users
            .iter()
            .find(|stored| stored.user_id == key)
            .cloned())
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("visibility never computes a budget reset"))
    }
}

// runs a statement once the permitted teams are read, so the scope and the recheck see different rows
pub(super) struct RacingStore {
    pub(super) inner: PostgresVisibility,
    pub(super) pool: Pool,
    pub(super) after_teams: &'static str,
}

#[async_trait]
impl VisibilityStore for RacingStore {
    async fn owners(&self, lookup_id: &str) -> Result<Vec<OwnerPair>, VisibilityError> {
        self.inner.owners(lookup_id).await
    }

    async fn payload_rows(
        &self,
        query: &PayloadQuery<'_>,
    ) -> Result<Vec<PayloadRow>, VisibilityError> {
        self.inner.payload_rows(query).await
    }

    async fn team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, VisibilityError> {
        self.inner.team(team_id).await
    }

    async fn teams(&self, team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError> {
        let teams = self.inner.teams(team_ids).await?;
        execute(&self.pool, self.after_teams).await;
        Ok(teams)
    }
}
