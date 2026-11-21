use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::user_repository::projection::user_record_to_model;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::management_admins::ports::{AdminUserReads, TeamReadError, TeamReads, UserReadError};
use crate::management_admins::scope::AdminCaller;

const FIXTURE_TIME: &str = "2026-01-01T00:00:00+00:00";

pub(crate) fn principal(user_id: Option<&str>, roles: Vec<Role>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: user_id.unwrap_or("service-account").to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles,
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

pub(crate) fn caller(user_id: Option<&str>) -> AdminCaller<'_> {
    AdminCaller {
        user_id,
        proxy_admin: false,
    }
}

fn membership(user_id: &str, organization_id: &str, user_role: Option<&str>) -> JsonData {
    JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "organization_id".to_owned(),
            JsonData::String(organization_id.to_owned()),
        ),
        (
            "user_role".to_owned(),
            user_role.map_or(JsonData::Null, |role| JsonData::String(role.to_owned())),
        ),
        (
            "created_at".to_owned(),
            JsonData::String(FIXTURE_TIME.to_owned()),
        ),
        (
            "updated_at".to_owned(),
            JsonData::String(FIXTURE_TIME.to_owned()),
        ),
    ]))
}

pub(crate) fn user_row(
    user_id: &str,
    teams: &[&str],
    memberships: &[(&str, Option<&str>)],
) -> LiteLLMUserTable {
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
        (
            "organization_memberships".to_owned(),
            JsonData::Array(
                memberships
                    .iter()
                    .map(|(organization_id, user_role)| {
                        membership(user_id, organization_id, *user_role)
                    })
                    .collect(),
            ),
        ),
    ]));
    user_record_to_model(Some(&record))
        .expect("the fixture user row is readable")
        .expect("the fixture user row is present")
}

pub(crate) fn user(user_id: &str, teams: &[&str]) -> LiteLLMUserTable {
    user_row(user_id, teams, &[])
}

pub(crate) fn org_user(user_id: &str, memberships: &[(&str, Option<&str>)]) -> LiteLLMUserTable {
    user_row(user_id, &[], memberships)
}

fn team_object(team_id: &str, members: &[(&str, &str)]) -> JsonData {
    let roster: Vec<JsonData> = members
        .iter()
        .map(|(user_id, role)| {
            JsonData::Object(BTreeMap::from([
                (
                    "user_id".to_owned(),
                    JsonData::String((*user_id).to_owned()),
                ),
                ("role".to_owned(), JsonData::String((*role).to_owned())),
            ]))
        })
        .collect();
    JsonData::Object(BTreeMap::from([
        ("team_id".to_owned(), JsonData::String(team_id.to_owned())),
        ("members_with_roles".to_owned(), JsonData::Array(roster)),
    ]))
}

fn parse_record(record: &JsonData) -> TeamRecord {
    let text = serde_json::to_string(record).expect("the fixture record serializes");
    TeamRecord::parse(text.as_bytes())
        .expect("the fixture record parses")
        .expect("the fixture record is an object")
}

pub(crate) fn team_row(team_id: &str, members: &[(&str, &str)]) -> TeamRecord {
    parse_record(&team_object(team_id, members))
}

pub(crate) fn invalid_team() -> TeamRecord {
    let text = r#"{"team_id":"broken","members_with_roles":"{broken"}"#;
    TeamRecord::parse(text.as_bytes())
        .expect("the fixture record parses")
        .expect("the fixture record is an object")
}

#[derive(Clone)]
pub(crate) struct ReadCall {
    pub(crate) user_id: String,
    pub(crate) user_id_upsert: bool,
    pub(crate) cache: Arc<dyn UserCache>,
}

#[derive(Default)]
pub(crate) struct FakeUsers {
    calls: Mutex<Vec<ReadCall>>,
    fresh: Mutex<Vec<Arc<dyn UserCache>>>,
    users: BTreeMap<String, LiteLLMUserTable>,
    failures: Mutex<Vec<UserReadError>>,
}

impl FakeUsers {
    pub(crate) fn new(users: Vec<(&str, LiteLLMUserTable)>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fresh: Mutex::new(Vec::new()),
            users: users
                .into_iter()
                .map(|(user_id, user)| (user_id.to_owned(), user))
                .collect(),
            failures: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn fail(&self, error: UserReadError) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(error);
    }

    pub(crate) fn calls(&self) -> Vec<ReadCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn fresh(&self) -> Vec<Arc<dyn UserCache>> {
        self.fresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl AdminUserReads for FakeUsers {
    fn fresh_cache(&self) -> Result<Arc<dyn UserCache>, UserReadError> {
        let cache: Arc<dyn UserCache> = Arc::new(FakeCache::new(60.0_f64));
        self.fresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(&cache));
        Ok(cache)
    }

    async fn user_object(
        &self,
        request: UserLookupRequest<'_>,
        cache: Arc<dyn UserCache>,
    ) -> Result<Option<LiteLLMUserTable>, UserReadError> {
        let user_id = request.user_id.unwrap_or_default().to_owned();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ReadCall {
                user_id: user_id.clone(),
                user_id_upsert: request.user_id_upsert,
                cache,
            });
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if !failures.is_empty() {
            return Err(failures.remove(0));
        }
        drop(failures);
        Ok(self.users.get(&user_id).cloned())
    }
}

pub(crate) struct FakeCache {
    cached: Mutex<Option<LiteLLMUserTable>>,
    ttl: f64,
}

impl FakeCache {
    pub(crate) fn new(ttl: f64) -> Self {
        Self {
            cached: Mutex::new(None),
            ttl,
        }
    }
}

#[async_trait]
impl UserCache for FakeCache {
    async fn read(&self, _key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self
            .cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    async fn write(
        &self,
        _key: &str,
        user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        *self.cached.lock().unwrap_or_else(PoisonError::into_inner) = Some(user.clone());
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        self.ttl
    }
}

pub(crate) struct FakeTeams {
    requested: Mutex<Vec<Vec<String>>>,
    records: Vec<TeamRecord>,
    fail: AtomicBool,
}

impl FakeTeams {
    pub(crate) fn new(records: Vec<TeamRecord>) -> Self {
        Self {
            requested: Mutex::new(Vec::new()),
            records,
            fail: AtomicBool::new(false),
        }
    }

    pub(crate) fn failing() -> Self {
        let teams = Self::new(Vec::new());
        teams.fail.store(true, Ordering::SeqCst);
        teams
    }

    pub(crate) fn requested(&self) -> Vec<Vec<String>> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Default for FakeTeams {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[async_trait]
impl TeamReads for FakeTeams {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError> {
        self.requested
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_ids.to_vec());
        if self.fail.load(Ordering::SeqCst) {
            return Err(TeamReadError::Database(DbError::NoDbConnected));
        }
        Ok(self.records.clone())
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
