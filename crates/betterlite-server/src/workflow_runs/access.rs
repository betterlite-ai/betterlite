use betterlite_auth::auth_identity::records::Principal;

use crate::tracing_routes::access::{TraceAuth, TraceRole};
use crate::workflow_runs::error::RunsError;
use crate::workflow_runs::models::WorkflowRunRow;
use crate::workflow_runs::store::WorkflowRunStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCaller {
    pub role: TraceRole,
    // the hashed key token; None for the master key and for a caller that has no key
    pub key: Option<String>,
}

impl RunCaller {
    #[must_use]
    pub fn from_principal(principal: &Principal) -> Self {
        let auth = TraceAuth::from_principal(principal);
        Self {
            role: auth.role,
            key: auth.token.filter(|token| !token.is_empty()),
        }
    }

    #[must_use]
    pub const fn has_admin_view(&self) -> bool {
        match self.role {
            TraceRole::Admin | TraceRole::AdminViewOnly => true,
            TraceRole::Other => false,
        }
    }

    // None for a caller that reads every run, the caller itself otherwise
    #[must_use]
    pub const fn read_scope(&self) -> Option<&Self> {
        if self.has_admin_view() {
            None
        } else {
            Some(self)
        }
    }
}

// A None caller skips the check, and so does a full PROXY_ADMIN; anyone else must own the run.
pub fn ensure_owner(
    run: WorkflowRunRow,
    caller: Option<&RunCaller>,
) -> Result<WorkflowRunRow, RunsError> {
    let Some(scoped) = caller else {
        return Ok(run);
    };
    if scoped.role == TraceRole::Admin {
        return Ok(run);
    }
    let owned = matches!((&scoped.key, &run.created_by), (Some(key), Some(owner)) if key == owner);
    if owned {
        Ok(run)
    } else {
        Err(RunsError::NotFound { run_id: run.run_id })
    }
}

pub async fn require_run(
    store: &dyn WorkflowRunStore,
    run_id: &str,
    caller: Option<&RunCaller>,
) -> Result<WorkflowRunRow, RunsError> {
    let run = store
        .find_unique(run_id)
        .await?
        .ok_or_else(|| RunsError::NotFound {
            run_id: run_id.to_owned(),
        })?;
    ensure_owner(run, caller)
}
