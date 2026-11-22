use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use axum::http::StatusCode;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use betterlite_db::team_rows::team::TeamBase;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::memory_records::access::TeamRows;
use crate::memory_records::caller::MemoryCaller;
use crate::memory_records::error::MemoryHttpError;
use crate::memory_records::filter::MemoryWhere;
use crate::memory_records::models::MemoryRow;
use crate::memory_records::store::{MemoryLookupError, MemoryRows};
use crate::team_access::access::{OrgRoles, TeamAccess};
use crate::team_access::error::TeamAccessError;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("the captured log lines are UTF-8")
    }
}

impl io::Write for Captured {
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

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(captured.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(crate) fn store_failure() -> TableError {
    TableError::Database(DbError::NoDbConnected)
}

pub(crate) fn row(
    memory_id: &str,
    key: &str,
    user_id: Option<&str>,
    team_id: Option<&str>,
) -> MemoryRow {
    MemoryRow {
        memory_id: memory_id.to_owned(),
        key: key.to_owned(),
        value: "value".to_owned(),
        metadata: None,
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        created_at: None,
        created_by: None,
        updated_at: None,
        updated_by: None,
    }
}

pub(crate) fn caller(
    user_id: Option<&str>,
    team_id: Option<&str>,
    user_role: Option<UserRole>,
) -> MemoryCaller {
    MemoryCaller {
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
        user_role,
        service_account: false,
        admin_view: matches!(
            user_role,
            Some(UserRole::ProxyAdmin | UserRole::ProxyAdminViewer)
        ),
    }
}

pub(crate) fn auth(
    user_id: Option<&str>,
    team_id: Option<&str>,
    user_role: Option<&str>,
) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    if let Some(owner) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(owner.to_owned()));
    }
    if let Some(scope) = team_id {
        fields.insert("team_id".to_owned(), JsonData::String(scope.to_owned()));
    }
    if let Some(role) = user_role {
        fields.insert("user_role".to_owned(), JsonData::String(role.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the case auth builds")
}

pub(crate) fn team(
    team_id: &str,
    organization_id: Option<&str>,
    admins: &[&str],
) -> TeamBase<String> {
    let owned_id = team_id.to_owned();
    let organization = organization_id.map(str::to_owned);
    let members_with_roles = admins
        .iter()
        .map(|user| Member {
            base: MemberBase {
                user_id: Some((*user).to_owned()),
                user_email: None,
            },
            role: MemberRole::Admin,
        })
        .collect();
    TeamBase {
        team_alias: None,
        team_id: owned_id,
        organization_id: organization,
        admins: Vec::new(),
        members: Vec::new(),
        members_with_roles,
        team_member_permissions: None,
        metadata: None,
        tpm_limit: None,
        rpm_limit: None,
        tpd_limit: None,
        max_budget: None,
        soft_budget: None,
        budget_duration: None,
        budget_limits: None,
        models: Vec::new(),
        blocked: false,
        router_settings: None,
        access_group_ids: None,
        default_team_member_models: None,
    }
}

pub(crate) struct CaseOrgRoles {
    pub(crate) admins: Vec<(String, String)>,
    pub(crate) fail: bool,
}

#[async_trait]
impl OrgRoles for CaseOrgRoles {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        if self.fail {
            return Err(TeamAccessError::Denied);
        }
        Ok(self
            .admins
            .iter()
            .any(|(user, organization)| user == user_id && organization == organization_id))
    }
}

pub(crate) fn access(admins: Vec<(String, String)>, fail: bool) -> TeamAccess {
    TeamAccess {
        org_roles: Arc::new(CaseOrgRoles { admins, fail }),
    }
}

pub(crate) fn org_admins(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(user, organization)| ((*user).to_owned(), (*organization).to_owned()))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowsFailure {
    Serves,
    StoreError,
    Forbidden,
}

pub(crate) struct FakeRows {
    pub(crate) answer: Option<MemoryRow>,
    pub(crate) failure: RowsFailure,
    pub(crate) seen: Mutex<Vec<MemoryWhere>>,
}

impl FakeRows {
    pub(crate) fn new(answer: Option<MemoryRow>) -> Self {
        Self {
            answer,
            failure: RowsFailure::Serves,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            answer: None,
            failure: RowsFailure::StoreError,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn forbidden() -> Self {
        Self {
            answer: None,
            failure: RowsFailure::Forbidden,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn filters(&self) -> Vec<MemoryWhere> {
        lock(&self.seen).clone()
    }
}

#[async_trait]
impl MemoryRows for FakeRows {
    async fn newest_memory_row(
        &self,
        filter: &MemoryWhere,
    ) -> Result<Option<MemoryRow>, MemoryLookupError> {
        lock(&self.seen).push(filter.clone());
        match self.failure {
            RowsFailure::Serves => Ok(self.answer.clone()),
            RowsFailure::StoreError => Err(MemoryLookupError::Rows(store_failure())),
            RowsFailure::Forbidden => Err(MemoryLookupError::Http(MemoryHttpError {
                status: StatusCode::FORBIDDEN,
                detail: "the row source refused the lookup".to_owned(),
            })),
        }
    }
}

pub(crate) struct FakeTeams {
    pub(crate) team: Option<TeamBase<String>>,
    pub(crate) fail: bool,
    pub(crate) seen: Mutex<Vec<String>>,
}

impl FakeTeams {
    pub(crate) fn new(team: Option<TeamBase<String>>) -> Self {
        Self {
            team,
            fail: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            team: None,
            fail: true,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn lookups(&self) -> Vec<String> {
        lock(&self.seen).clone()
    }
}

#[async_trait]
impl TeamRows for FakeTeams {
    async fn find_team(&self, team_id: &str) -> Result<Option<TeamBase<String>>, TeamQueryError> {
        lock(&self.seen).push(team_id.to_owned());
        if self.fail {
            return Err(TeamQueryError::from(DbError::NoDbConnected));
        }
        Ok(self.team.clone())
    }
}
