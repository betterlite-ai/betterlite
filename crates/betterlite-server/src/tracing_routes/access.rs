use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_logging::otlp_normalize::SpanRow;
use betterlite_logging::trace_types::TraceScope;
use thiserror::Error;

use crate::tracing_routes::receiver::TraceRuntime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub team_id: String,
    pub api_key_hash: String,
    pub org_id: String,
}

impl Tenant {
    pub fn stamp_rows(&self, rows: &mut [SpanRow]) {
        for row in rows.iter_mut() {
            row.team_id.clone_from(&self.team_id);
            row.api_key_hash.clone_from(&self.api_key_hash);
            row.resource_attributes
                .insert("litellm.team_id".to_owned(), self.team_id.clone());
            row.resource_attributes
                .insert("litellm.api_key_hash".to_owned(), self.api_key_hash.clone());
            row.resource_attributes
                .insert("litellm.org_id".to_owned(), self.org_id.clone());
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TraceRole {
    Admin,
    AdminViewOnly,
    #[default]
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceAuth {
    pub role: TraceRole,
    pub team_id: Option<String>,
    pub token: Option<String>,
    pub org_id: Option<String>,
    pub project_id: Option<String>,
}

impl TraceAuth {
    #[must_use]
    pub fn from_principal(principal: &Principal) -> Self {
        let role = if principal.roles.contains(&Role::PlatformAdmin) {
            TraceRole::Admin
        } else if principal.roles.contains(&Role::PlatformViewer) {
            TraceRole::AdminViewOnly
        } else {
            TraceRole::Other
        };
        Self {
            role,
            team_id: principal.teams.first().map(|team| team.id.clone()),
            token: principal
                .credential_ref
                .token_id
                .clone()
                .or_else(|| principal.credential_ref.key_id.clone()),
            org_id: principal
                .organization
                .as_ref()
                .map(|organization| organization.id.clone()),
            project_id: principal.project.as_ref().map(|project| project.id.clone()),
        }
    }
}

#[derive(Debug, Error)]
pub enum AccessError {
    #[error("Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.")]
    Unavailable,
    #[error("Not allowed to view agent traces")]
    NotAllowedToView,
    #[error("Not allowed to ingest agent traces")]
    NotAllowedToIngest,
}

pub struct TraceAccessContext {
    pub receiver: Option<Arc<dyn TraceRuntime>>,
    pub read_scope: Option<TraceScope>,
    pub write_tenant: Option<Tenant>,
}

impl TraceAccessContext {
    pub fn reader(&self) -> Result<(&Arc<dyn TraceRuntime>, &TraceScope), AccessError> {
        let receiver = self.receiver.as_ref().ok_or(AccessError::Unavailable)?;
        let scope = self
            .read_scope
            .as_ref()
            .ok_or(AccessError::NotAllowedToView)?;
        Ok((receiver, scope))
    }

    pub fn writer(&self) -> Result<(&Arc<dyn TraceRuntime>, &Tenant), AccessError> {
        match (&self.write_tenant, &self.receiver) {
            (Some(tenant), Some(receiver)) => Ok((receiver, tenant)),
            (None, _) => Err(AccessError::NotAllowedToIngest),
            (_, None) => Err(AccessError::Unavailable),
        }
    }
}

#[must_use]
pub fn provide_trace_access(
    auth: &TraceAuth,
    receiver: Option<Arc<dyn TraceRuntime>>,
) -> TraceAccessContext {
    let tenant = Tenant {
        team_id: auth.team_id.clone().unwrap_or_default(),
        api_key_hash: auth.token.clone().unwrap_or_default(),
        org_id: auth.org_id.clone().unwrap_or_default(),
    };
    let (read_scope, write_tenant) = trace_scopes(auth, tenant);
    TraceAccessContext {
        receiver,
        read_scope,
        write_tenant,
    }
}

fn trace_scopes(auth: &TraceAuth, tenant: Tenant) -> (Option<TraceScope>, Option<Tenant>) {
    match auth.role {
        TraceRole::Admin => (
            Some(TraceScope {
                team_ids: Vec::new(),
                api_key_hash: String::new(),
            }),
            Some(tenant),
        ),
        TraceRole::AdminViewOnly => (
            Some(TraceScope {
                team_ids: Vec::new(),
                api_key_hash: String::new(),
            }),
            None,
        ),
        TraceRole::Other => match (&auth.team_id, &auth.token) {
            (Some(team_id), _) => (
                Some(TraceScope {
                    team_ids: vec![team_id.clone()],
                    api_key_hash: String::new(),
                }),
                Some(tenant),
            ),
            (None, Some(token)) => (
                Some(TraceScope {
                    team_ids: vec![String::new()],
                    api_key_hash: token.clone(),
                }),
                Some(tenant),
            ),
            (None, None) => (None, Some(tenant)),
        },
    }
}

pub trait TraceClock: Send + Sync {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl TraceClock for SystemClock {
    fn now_ms(&self) -> i64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
    }
}
