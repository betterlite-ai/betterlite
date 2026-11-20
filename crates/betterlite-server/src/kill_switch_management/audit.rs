use betterlite_config::sections::settings::JsonValue;
use betterlite_core::agents::{AgentKillSwitchMethod, AgentKillSwitchResult};
use serde::Serialize;
use thiserror::Error;

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::sink::AuditClock;

pub struct KillSwitchAuditContext<'ctx> {
    pub caller: &'ctx CallerIdentity,
    pub litellm_proxy_admin_name: Option<&'ctx str>,
    pub clock: &'ctx dyn AuditClock,
    pub id: String,
}

#[derive(Debug, Error)]
pub enum KillSwitchAuditError {
    #[error("the kill switch result could not be serialized: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct ResultValues<'ctx> {
    agent_id: &'ctx str,
    url: &'ctx str,
    method: &'ctx AgentKillSwitchMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_body: Option<&'ctx str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'ctx str>,
}

pub fn build_kill_switch_audit_log(
    result: &AgentKillSwitchResult,
    context: KillSwitchAuditContext<'_>,
) -> Result<AuditLogRecord, KillSwitchAuditError> {
    let wire = ResultValues {
        agent_id: result.agent_id.as_str(),
        url: result.url.as_str(),
        method: &result.method,
        status_code: result.status_code,
        response_body: result.response_body.as_deref(),
        error: result.error.as_deref(),
    };
    let updated_values = serde_json::to_string(&wire)?;
    Ok(AuditLogRecord {
        id: context.id,
        updated_at: context.clock.now(),
        changed_by: get_audit_log_changed_by(
            None,
            context.caller,
            context.litellm_proxy_admin_name,
        ),
        changed_by_api_key: context.caller.api_key.clone(),
        action: AuditAction::KillSwitchFired,
        table_name: AuditTable::Agent,
        object_id: result.agent_id.clone(),
        before_value: None,
        updated_values: Some(JsonValue::String(updated_values)),
    })
}

mod tests;
