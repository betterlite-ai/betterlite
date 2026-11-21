use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;

use crate::model_write_fields::error::{Detail, ModelWriteError, Rejection};
use crate::model_write_fields::member::{MemberRouterWrite, MemberWrite};
use crate::model_write_fields::teams::TeamRows;
use crate::team_access::access::{OrgRoles, TeamAccess};
use crate::team_access::caller::TeamCaller;
use crate::team_access::error::TeamAccessError;

pub(super) const SALT: &str = "model-write-fields-salt";
// LiteLLM's encrypt_value_helper("shared-openai") under LITELLM_SALT_KEY=model-write-fields-salt
pub(super) const SEALED_NAME: &str =
    "sERBFztHSkMcwAVhLYCuQALpnrBBFzqx1hX93kY7LtB8j8CSkMBDIDqJUHdGXfXY0LLBGkU=";

pub(super) fn caller(user_id: &str, role: Option<UserRole>) -> TeamCaller {
    TeamCaller {
        user_id: Some(user_id.to_owned()),
        user_role: role,
        team_id: Some("key-team".to_owned()),
        team_service_account: false,
    }
}

pub(super) fn team(team_id: &str) -> LiteLLMTeamTable {
    let row = format!(
        r#"{{"team_id": "{team_id}", "organization_id": "org-1", "members_with_roles": [{{"user_id": "admin-1", "role": "admin"}}, {{"user_id": "member-1", "role": "user"}}]}}"#
    );
    serde_json::from_str(&row).unwrap()
}

#[derive(Default)]
pub(super) struct FakeTeams {
    pub(super) teams: Vec<LiteLLMTeamTable>,
    pub(super) reads: Mutex<Vec<String>>,
}

impl FakeTeams {
    pub(super) fn holding(team_ids: &[&str]) -> Self {
        Self {
            teams: team_ids.iter().map(|team_id| team(team_id)).collect(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl TeamRows for FakeTeams {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, TeamQueryError> {
        self.reads.lock().unwrap().push(team_id.to_owned());
        Ok(self
            .teams
            .iter()
            .find(|stored| stored.fields.base.team_id == team_id)
            .cloned())
    }
}

// every caller counts as an organization admin, so a lookup would grant what TEAM_ADMIN_ONLY must not
#[derive(Default)]
pub(super) struct CountedOrgRoles {
    pub(super) lookups: Mutex<Vec<String>>,
}

#[async_trait]
impl OrgRoles for CountedOrgRoles {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        let lookup = format!("{user_id} {organization_id}");
        self.lookups.lock().unwrap().push(lookup);
        Ok(true)
    }
}

pub(super) fn team_access(org_roles: &Arc<CountedOrgRoles>) -> TeamAccess {
    let roles: Arc<dyn OrgRoles> = Arc::<CountedOrgRoles>::clone(org_roles);
    TeamAccess { org_roles: roles }
}

#[derive(Debug, thiserror::Error)]
#[error("the member router policy refused")]
pub(super) struct PolicyRefusal;

pub(super) struct RecordingPolicy {
    pub(super) refuse: bool,
    pub(super) calls: Mutex<Vec<String>>,
}

#[async_trait]
impl MemberRouterWrite for RecordingPolicy {
    type Patch = String;
    type Stored = String;
    type Grant = String;
    type Refusal = PolicyRefusal;

    async fn authorize(
        &self,
        caller: &TeamCaller,
        team: &LiteLLMTeamTable,
        write: MemberWrite<'_, String, String>,
    ) -> Result<String, PolicyRefusal> {
        let kind = match write {
            MemberWrite::Create { deployment } => format!("create {deployment}"),
            MemberWrite::Update { patch, existing } => format!("update {patch} over {existing}"),
        };
        let call = format!(
            "{} {} {kind}",
            team.fields.base.team_id,
            caller.user_id.clone().unwrap_or_default()
        );
        self.calls.lock().unwrap().push(call);
        if self.refuse {
            return Err(PolicyRefusal);
        }
        Ok(kind)
    }
}

#[derive(Default)]
pub(super) struct RecordedLogs {
    pub(super) entries: Mutex<Vec<(Level, String)>>,
}

impl SecretDiagnostics for RecordedLogs {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        self.entries.lock().unwrap().push((level, event.message()));
    }
}

pub(super) fn cipher() -> StoredCipher<OsNonces, RecordedLogs> {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: RecordedLogs::default(),
    }
}

pub(super) fn rejection(error: &ModelWriteError) -> Rejection {
    error.rejection().unwrap()
}

pub(super) fn proxy_fields(error: &ModelWriteError) -> (u16, String, Option<String>, String) {
    let Rejection {
        status,
        detail: Detail::Proxy(exception),
    } = rejection(error)
    else {
        panic!("{error} is not a ProxyException");
    };
    let ProxyException {
        error_type,
        param,
        code,
        ..
    } = exception;
    (status.as_u16(), error_type, param, code)
}
