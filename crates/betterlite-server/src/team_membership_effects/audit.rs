use std::sync::Arc;

use betterlite_db::team_rows::member::Member;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use serde::Serialize;

use crate::audit::service::AuditService;
use crate::audit_events::callbacks::{AuditCallbackFuture, AuditTaskSpawner};
use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::sink::ObjectAuditLog;

mod tests;

pub struct TeamMembershipAudit<'entry> {
    pub team_id: &'entry str,
    pub team_alias: Option<&'entry str>,
    pub before_members: &'entry [Member],
    pub after_members: &'entry [Member],
    pub caller: &'entry CallerIdentity,
    pub litellm_proxy_admin_name: Option<&'entry str>,
}

#[derive(Serialize)]
struct MembersAuditValue<'value> {
    team_alias: Option<&'value str>,
    members_with_roles: &'value [Member],
}

// LiteLLM puts a tuple of model dumps under members_with_roles; a top-level array is not a JSON object and the audit columns hold objects
pub fn members_audit_value(
    team_alias: Option<&str>,
    members: &[Member],
) -> Result<String, PrivateJsonError> {
    safe_dumps(
        &MembersAuditValue {
            team_alias,
            members_with_roles: members,
        },
        DEFAULT_MAX_RECURSE_DEPTH,
        None,
    )
}

// ast-grep-ignore: no-long-names - LiteLLM's function name, so every route task that calls it reads the same as its Python
pub fn schedule_team_membership_audit_log(
    service: &Arc<AuditService>,
    spawner: &Arc<dyn AuditTaskSpawner>,
    entry: &TeamMembershipAudit<'_>,
) -> Result<(), PrivateJsonError> {
    if !service.settings.config.is_audit_logging_enabled(None) {
        return Ok(());
    }
    if entry.before_members == entry.after_members {
        return Ok(());
    }
    // LiteLLM evaluates both values before asyncio.create_task, so a failure reaches the route and no task exists
    let before_value = members_audit_value(entry.team_alias, entry.before_members)?;
    let after_value = members_audit_value(entry.team_alias, entry.after_members)?;
    let shared_service = Arc::clone(service);
    let caller = entry.caller.clone();
    let team_id = entry.team_id.to_owned();
    let litellm_proxy_admin_name = entry.litellm_proxy_admin_name.map(str::to_owned);
    let task: AuditCallbackFuture = Box::pin(async move {
        shared_service
            .create_object_audit_log(ObjectAuditLog {
                object_id: &team_id,
                action: AuditAction::Updated,
                litellm_changed_by: None,
                caller: &caller,
                litellm_proxy_admin_name: litellm_proxy_admin_name.as_deref(),
                table_name: AuditTable::Team,
                before_value: Some(&before_value),
                after_value: Some(&after_value),
            })
            .await;
    });
    spawner.spawn(task);
    Ok(())
}
