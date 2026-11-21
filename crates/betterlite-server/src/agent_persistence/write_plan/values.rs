use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_auth::agent_identity::managed_write::IdentityUpdate;
use betterlite_auth::agent_identity::managed_write::ManagedWriteInput;
use betterlite_auth::agent_policy::AgentExecutionMode;
use betterlite_auth::agent_policy::AgentResponse as PolicyAgent;
use betterlite_auth::identity::EntraIdentityConfig;
use betterlite_auth::identity::ExecutionMode;
use betterlite_core::agents::card::AgentCard;
use betterlite_core::agents::json::AgentJson;

use crate::agent_credentials::params::parse_agent_litellm_params;
// ast-grep-ignore: no-long-names
use crate::agent_credentials::restore::restore_redacted_litellm_params;
use crate::agent_rows::codec::StoredAgent;

use super::super::error::AgentPersistenceError;
use super::super::json_text::safe_json_text;
use super::PlanContext;

pub(super) fn managed_input(
    identity: Option<&Option<EntraIdentityConfig>>,
    execution_mode: Option<&ExecutionMode>,
    enabled: Option<bool>,
) -> Result<ManagedWriteInput, AgentPersistenceError> {
    let update = match identity {
        None => IdentityUpdate::Absent,
        Some(None) => IdentityUpdate::Null,
        Some(Some(config)) => {
            let text = serde_json::to_string(config)?;
            IdentityUpdate::Value(serde_json::from_str(&text)?)
        }
    };
    Ok(ManagedWriteInput {
        identity: update,
        execution_mode: execution_mode.map(policy_mode),
        enabled,
    })
}

// B4772 reads no kill switch, and the core type leaves its secret out of the serialized form
pub(super) fn policy_agent(
    existing: Option<&StoredAgent>,
) -> Result<Option<PolicyAgent>, AgentPersistenceError> {
    let Some(stored) = existing else {
        return Ok(None);
    };
    let mut document: AgentJson = serde_json::to_value(&stored.agent)?;
    if let AgentJson::Object(fields) = &mut document {
        drop(fields.remove("kill_switch"));
    }
    let text = serde_json::to_string(&document)?;
    let agent: PolicyAgent = serde_json::from_str(&text)?;
    Ok(Some(agent))
}

pub(super) fn params_text(
    incoming: Option<&BTreeMap<String, AgentJson>>,
    existing: &BTreeMap<String, AgentJson>,
    context: &PlanContext<'_>,
) -> String {
    let empty = BTreeMap::new();
    let requested = incoming.unwrap_or(&empty);
    // ast-grep-ignore: no-long-names
    let restored = restore_redacted_litellm_params(requested, existing, context.masker);
    let value = AgentJson::Object(restored.into_iter().collect());
    safe_json_text(&value, context.max_json_depth)
}

pub(super) fn stored_params(stored: Option<&StoredAgent>) -> BTreeMap<String, AgentJson> {
    let Some(record) = stored else {
        return BTreeMap::new();
    };
    parse_agent_litellm_params(record.litellm_params_column.as_ref())
}

pub(super) fn card_text(
    card: Option<&AgentCard>,
    context: &PlanContext<'_>,
) -> Result<String, AgentPersistenceError> {
    let Some(present) = card else {
        return Ok(String::from("{}"));
    };
    let value = serde_json::to_value(present)?;
    Ok(safe_json_text(&value, context.max_json_depth))
}

pub(super) fn headers_text(headers: Option<&BTreeMap<String, String>>, max_depth: usize) -> String {
    let Some(pairs) = headers else {
        return String::from("{}");
    };
    let value = AgentJson::Object(
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), AgentJson::String(value.clone())))
            .collect(),
    );
    safe_json_text(&value, max_depth)
}

pub(super) fn deduplicated(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut kept = Vec::new();
    for value in values {
        if seen.insert(value.as_str()) {
            kept.push(value.clone());
        }
    }
    kept
}

const fn policy_mode(mode: &ExecutionMode) -> AgentExecutionMode {
    match mode {
        ExecutionMode::Autonomous => AgentExecutionMode::Autonomous,
        ExecutionMode::Delegated => AgentExecutionMode::Delegated,
        ExecutionMode::Both => AgentExecutionMode::Both,
    }
}

pub(super) const fn mode_text(mode: &AgentExecutionMode) -> &'static str {
    match mode {
        AgentExecutionMode::Autonomous => "autonomous",
        AgentExecutionMode::Delegated => "delegated",
        AgentExecutionMode::Both => "both",
    }
}
