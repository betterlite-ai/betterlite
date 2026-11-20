use std::collections::BTreeMap;

use betterlite_auth::agent_identity::managed_write::managed_write_fields;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryWrite;
use betterlite_auth::agent_identity::write_fields::IdentityRelationWrite;
use betterlite_auth::agent_identity::write_fields::ManagedWriteFields;
use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::PatchAgentRequest;
use betterlite_redact::structured::SensitiveDataMasker;
use chrono::DateTime;
use chrono::NaiveDateTime;
use chrono::Utc;
use uuid::Uuid;

use crate::agent_credentials::kill_switch::serialize_agent_kill_switch;
use crate::agent_rows::codec::StoredAgent;

use self::values::card_text;
use self::values::create_headers;
use self::values::deduplicated;
use self::values::managed_input;
use self::values::mode_text;
use self::values::params_text;
use self::values::patched_card;
use self::values::patched_groups;
use self::values::patched_headers;
use self::values::patched_kill_switch;
use self::values::patched_params;
use self::values::policy_agent;
use self::values::replace_headers;
use self::values::stored_params;

use super::error::AgentPersistenceError;

mod tests;
mod values;

pub struct AgentColumns {
    pub agent_name: Option<String>,
    pub litellm_params: Option<String>,
    pub agent_card_params: Option<String>,
    pub static_headers: Option<String>,
    pub extra_headers: Option<Vec<String>>,
    pub kill_switch: Option<String>,
    pub access_group_ids: Option<Vec<String>>,
    pub tpm_limit: Option<Option<i64>>,
    pub rpm_limit: Option<Option<i64>>,
    pub session_tpm_limit: Option<Option<i64>>,
    pub session_rpm_limit: Option<Option<i64>>,
    pub enabled: Option<bool>,
    pub execution_mode: Option<String>,
    pub identity_managed: Option<bool>,
    pub created_by: Option<String>,
    pub created_at: Option<NaiveDateTime>,
    pub updated_by: Option<String>,
    pub updated_at: Option<NaiveDateTime>,
}

pub struct PlanContext<'masker> {
    pub masker: &'masker SensitiveDataMasker,
    pub max_json_depth: usize,
    pub now: DateTime<Utc>,
}

pub struct AgentWritePlan {
    pub columns: AgentColumns,
    pub identity: Option<IdentityRelationWrite>,
    pub history: Option<IdentityHistoryWrite>,
}

pub fn plan_create(
    agent: &AgentConfig,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentWritePlan, AgentPersistenceError> {
    let identity = agent.identity.as_ref().map(Option::as_ref);
    let input = managed_input(identity, agent.execution_mode.as_ref(), agent.enabled)?;
    let managed = managed_write_fields(&input, None, Uuid::new_v4)
        .map_err(AgentPersistenceError::Identity)?;
    let columns = create_columns(agent, &managed, actor, context)?;
    Ok(AgentWritePlan {
        columns,
        identity: managed.identity,
        history: managed.retired_identities,
    })
}

fn create_columns(
    agent: &AgentConfig,
    managed: &ManagedWriteFields,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentColumns, AgentPersistenceError> {
    let empty = BTreeMap::new();
    let kill_switch =
        serialize_agent_kill_switch(agent.kill_switch.as_ref().and_then(Option::as_ref), None)?;
    Ok(AgentColumns {
        agent_name: Some(agent.agent_name.clone()),
        litellm_params: Some(params_text(agent.litellm_params.as_ref(), &empty, context)),
        agent_card_params: Some(card_text(agent.agent_card_params.as_ref(), context)?),
        static_headers: create_headers(agent, context),
        extra_headers: agent.extra_headers.clone().flatten(),
        kill_switch: Some(kill_switch),
        access_group_ids: agent
            .access_group_ids
            .clone()
            .flatten()
            .map(|ids| deduplicated(&ids)),
        tpm_limit: agent.tpm_limit.filter(Option::is_some),
        rpm_limit: agent.rpm_limit.filter(Option::is_some),
        session_tpm_limit: agent.session_tpm_limit.filter(Option::is_some),
        session_rpm_limit: agent.session_rpm_limit.filter(Option::is_some),
        enabled: managed.enabled,
        execution_mode: managed
            .execution_mode
            .as_ref()
            .map(|mode| String::from(mode_text(mode))),
        identity_managed: managed.identity_managed,
        created_by: Some(String::from(actor)),
        created_at: Some(context.now.naive_utc()),
        updated_by: Some(String::from(actor)),
        updated_at: Some(context.now.naive_utc()),
    })
}

pub fn plan_replace(
    agent: &AgentConfig,
    existing: Option<&StoredAgent>,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentWritePlan, AgentPersistenceError> {
    let identity = agent.identity.as_ref().map(Option::as_ref);
    let input = managed_input(identity, agent.execution_mode.as_ref(), agent.enabled)?;
    let policy = policy_agent(existing)?;
    let managed = managed_write_fields(&input, policy.as_ref(), Uuid::new_v4)
        .map_err(AgentPersistenceError::Identity)?;
    let columns = replace_columns(agent, existing, &managed, actor, context)?;
    Ok(AgentWritePlan {
        columns,
        identity: managed.identity,
        history: managed.retired_identities,
    })
}

fn replace_columns(
    agent: &AgentConfig,
    existing: Option<&StoredAgent>,
    managed: &ManagedWriteFields,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentColumns, AgentPersistenceError> {
    let stored = stored_params(existing);
    let params = params_text(agent.litellm_params.as_ref(), &stored, context);
    let groups = agent.access_group_ids.clone().flatten().unwrap_or_default();
    let kill_switch = serialize_agent_kill_switch(
        agent.kill_switch.as_ref().and_then(Option::as_ref),
        existing.and_then(|record| record.kill_switch_column.as_ref()),
    )?;
    Ok(AgentColumns {
        agent_name: Some(agent.agent_name.clone()),
        litellm_params: Some(params),
        agent_card_params: Some(card_text(agent.agent_card_params.as_ref(), context)?),
        static_headers: Some(replace_headers(agent, context)),
        extra_headers: Some(agent.extra_headers.clone().flatten().unwrap_or_default()),
        kill_switch: Some(kill_switch),
        access_group_ids: Some(deduplicated(&groups)),
        tpm_limit: agent.tpm_limit.filter(Option::is_some),
        rpm_limit: agent.rpm_limit.filter(Option::is_some),
        session_tpm_limit: agent.session_tpm_limit.filter(Option::is_some),
        session_rpm_limit: agent.session_rpm_limit.filter(Option::is_some),
        enabled: managed.enabled,
        execution_mode: managed
            .execution_mode
            .as_ref()
            .map(|mode| String::from(mode_text(mode))),
        identity_managed: managed.identity_managed,
        created_by: None,
        created_at: None,
        updated_by: Some(String::from(actor)),
        updated_at: Some(context.now.naive_utc()),
    })
}

pub fn plan_patch(
    agent: &PatchAgentRequest,
    existing: &StoredAgent,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentWritePlan, AgentPersistenceError> {
    let identity = agent.identity.as_ref().map(Option::as_ref);
    let input = managed_input(identity, agent.execution_mode.as_ref(), agent.enabled)?;
    let policy = policy_agent(Some(existing))?;
    let managed = managed_write_fields(&input, policy.as_ref(), Uuid::new_v4)
        .map_err(AgentPersistenceError::Identity)?;
    let columns = patch_columns(agent, existing, &managed, actor, context)?;
    Ok(AgentWritePlan {
        columns,
        identity: managed.identity,
        history: managed.retired_identities,
    })
}

fn patch_columns(
    agent: &PatchAgentRequest,
    existing: &StoredAgent,
    managed: &ManagedWriteFields,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentColumns, AgentPersistenceError> {
    Ok(AgentColumns {
        agent_name: agent.agent_name.clone(),
        litellm_params: patched_params(agent, existing, context),
        agent_card_params: patched_card(agent, context)?,
        static_headers: patched_headers(agent, context),
        extra_headers: agent.extra_headers.clone().map(Option::unwrap_or_default),
        kill_switch: patched_kill_switch(agent, existing)?,
        access_group_ids: patched_groups(agent),
        tpm_limit: agent.tpm_limit,
        rpm_limit: agent.rpm_limit,
        session_tpm_limit: agent.session_tpm_limit,
        session_rpm_limit: agent.session_rpm_limit,
        enabled: managed.enabled,
        execution_mode: managed
            .execution_mode
            .as_ref()
            .map(|mode| String::from(mode_text(mode))),
        identity_managed: managed.identity_managed,
        created_by: None,
        created_at: None,
        updated_by: Some(String::from(actor)),
        updated_at: Some(context.now.naive_utc()),
    })
}
