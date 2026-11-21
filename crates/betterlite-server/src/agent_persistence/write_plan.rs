use std::collections::BTreeMap;

use betterlite_auth::agent_identity::managed_write::managed_write_fields;
use betterlite_auth::agent_identity::write_fields::IdentityHistoryWrite;
use betterlite_auth::agent_identity::write_fields::IdentityRelationWrite;
use betterlite_auth::identity::EntraIdentityConfig;
use betterlite_auth::identity::ExecutionMode;
use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::AgentObjectPermission;
use betterlite_core::agents::PatchAgentRequest;
use betterlite_core::agents::json::AgentJson;
use betterlite_redact::structured::SensitiveDataMasker;
use chrono::DateTime;
use chrono::NaiveDateTime;
use chrono::Utc;
use uuid::Uuid;

use crate::agent_rows::codec::StoredAgent;

use self::values::managed_input;
use self::values::mode_text;
use self::values::policy_agent;

use super::error::AgentPersistenceError;

mod sources;
mod tests;
mod values;

type ColumnResult = Result<Option<String>, AgentPersistenceError>;

pub(super) type Limits = [Option<Option<i64>>; 4];

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

#[derive(Clone, Copy)]
pub(super) enum ConfigMode {
    Create,
    Replace,
}

pub(super) trait ColumnSource: Send + Sync {
    fn identity(&self) -> Option<&Option<EntraIdentityConfig>>;
    fn execution_mode(&self) -> Option<&ExecutionMode>;
    fn enabled(&self) -> Option<bool>;
    fn permission(&self) -> Option<&AgentObjectPermission>;
    fn not_found(&self, agent_id: &str) -> String;
    fn params_map(&self) -> Option<&BTreeMap<String, AgentJson>>;
    fn name(&self) -> Option<String>;
    fn params(&self, row: Option<&StoredAgent>, context: &PlanContext<'_>) -> Option<String>;
    fn card(&self, context: &PlanContext<'_>) -> ColumnResult;
    fn headers(&self, context: &PlanContext<'_>) -> Option<String>;
    fn extra(&self) -> Option<Vec<String>>;
    fn kill_switch(&self, row: Option<&StoredAgent>) -> ColumnResult;
    fn groups(&self) -> Option<Vec<String>>;
    fn limits(&self) -> Limits;
    fn created_by(&self, actor: &str) -> Option<String>;
    fn created_at(&self, context: &PlanContext<'_>) -> Option<NaiveDateTime>;
}

pub(super) struct ConfigSource<'agent> {
    pub(super) agent: &'agent AgentConfig,
    pub(super) mode: ConfigMode,
}

pub(super) struct PatchSource<'agent> {
    pub(super) agent: &'agent PatchAgentRequest,
}

pub(super) fn agent_limits(agent: &AgentConfig) -> Limits {
    [
        agent.tpm_limit.filter(Option::is_some),
        agent.rpm_limit.filter(Option::is_some),
        agent.session_tpm_limit.filter(Option::is_some),
        agent.session_rpm_limit.filter(Option::is_some),
    ]
}

pub(super) fn plan_write(
    source: &dyn ColumnSource,
    row: Option<&StoredAgent>,
    actor: &str,
    context: &PlanContext<'_>,
) -> Result<AgentWritePlan, AgentPersistenceError> {
    let input = managed_input(source.identity(), source.execution_mode(), source.enabled())?;
    let policy = policy_agent(row)?;
    let managed = managed_write_fields(&input, policy.as_ref(), Uuid::new_v4)
        .map_err(AgentPersistenceError::Identity)?;
    let [tokens, requests, session_tokens, session_requests] = source.limits();
    let mode = managed.execution_mode.as_ref();
    let execution_mode = mode.map(mode_text).map(String::from);
    let columns = AgentColumns {
        agent_name: source.name(),
        litellm_params: source.params(row, context),
        agent_card_params: source.card(context)?,
        static_headers: source.headers(context),
        extra_headers: source.extra(),
        kill_switch: source.kill_switch(row)?,
        access_group_ids: source.groups(),
        tpm_limit: tokens,
        rpm_limit: requests,
        session_tpm_limit: session_tokens,
        session_rpm_limit: session_requests,
        enabled: managed.enabled,
        execution_mode,
        identity_managed: managed.identity_managed,
        created_by: source.created_by(actor),
        created_at: source.created_at(context),
        updated_by: Some(String::from(actor)),
        updated_at: Some(context.now.naive_utc()),
    };
    Ok(AgentWritePlan {
        columns,
        identity: managed.identity,
        history: managed.retired_identities,
    })
}
