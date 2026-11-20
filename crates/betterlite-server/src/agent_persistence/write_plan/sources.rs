use std::collections::BTreeMap;

use betterlite_auth::identity::EntraIdentityConfig;
use betterlite_auth::identity::ExecutionMode;
use betterlite_core::agents::AgentObjectPermission;
use betterlite_core::agents::json::AgentJson;
use chrono::NaiveDateTime;

use crate::agent_credentials::kill_switch::serialize_agent_kill_switch;
use crate::agent_rows::codec::StoredAgent;

use super::ColumnResult;
use super::ColumnSource;
use super::ConfigMode;
use super::ConfigSource;
use super::Limits;
use super::PatchSource;
use super::PlanContext;
use super::agent_limits;
use super::values::card_text;
use super::values::deduplicated;
use super::values::headers_text;
use super::values::params_text;
use super::values::stored_params;

impl ColumnSource for ConfigSource<'_> {
    fn identity(&self) -> Option<Option<&EntraIdentityConfig>> {
        self.agent.identity.as_ref().map(Option::as_ref)
    }

    fn execution_mode(&self) -> Option<&ExecutionMode> {
        self.agent.execution_mode.as_ref()
    }

    fn enabled(&self) -> Option<bool> {
        self.agent.enabled
    }

    fn permission(&self) -> Option<&AgentObjectPermission> {
        self.agent.object_permission.as_ref()
    }

    fn not_found(&self, agent_id: &str) -> String {
        format!("Agent not found, passed agent_id={agent_id}")
    }

    fn params_map(&self) -> Option<&BTreeMap<String, AgentJson>> {
        self.agent.litellm_params.as_ref()
    }

    fn name(&self) -> Option<String> {
        Some(self.agent.agent_name.clone())
    }

    fn params(&self, row: Option<&StoredAgent>, context: &PlanContext<'_>) -> Option<String> {
        let stored = stored_params(row);
        Some(params_text(self.params_map(), &stored, context))
    }

    fn card(&self, context: &PlanContext<'_>) -> ColumnResult {
        Ok(Some(card_text(
            self.agent.agent_card_params.as_ref(),
            context,
        )?))
    }

    fn headers(&self, context: &PlanContext<'_>) -> Option<String> {
        let headers = self.agent.static_headers.as_ref().and_then(Option::as_ref);
        if matches!(self.mode, ConfigMode::Replace) {
            return Some(headers_text(headers, context.max_json_depth));
        }
        let present = headers.filter(|pairs| !pairs.is_empty())?;
        Some(headers_text(Some(present), context.max_json_depth))
    }

    fn extra(&self) -> Option<Vec<String>> {
        let extra = self.agent.extra_headers.clone().flatten();
        match self.mode {
            ConfigMode::Create => extra,
            ConfigMode::Replace => Some(extra.unwrap_or_default()),
        }
    }

    fn kill_switch(&self, row: Option<&StoredAgent>) -> ColumnResult {
        let switch = self.agent.kill_switch.as_ref().and_then(Option::as_ref);
        let stored = row.and_then(|record| record.kill_switch_column.as_ref());
        Ok(Some(serialize_agent_kill_switch(switch, stored)?))
    }

    fn groups(&self) -> Option<Vec<String>> {
        let ids = self.agent.access_group_ids.clone().flatten();
        if matches!(self.mode, ConfigMode::Create) {
            return ids.map(|list| deduplicated(&list));
        }
        Some(deduplicated(&ids.unwrap_or_default()))
    }

    fn limits(&self) -> Limits {
        agent_limits(self.agent)
    }

    fn created_by(&self, actor: &str) -> Option<String> {
        match self.mode {
            ConfigMode::Create => Some(String::from(actor)),
            ConfigMode::Replace => None,
        }
    }

    fn created_at(&self, context: &PlanContext<'_>) -> Option<NaiveDateTime> {
        match self.mode {
            ConfigMode::Create => Some(context.now.naive_utc()),
            ConfigMode::Replace => None,
        }
    }
}

impl ColumnSource for PatchSource<'_> {
    fn identity(&self) -> Option<Option<&EntraIdentityConfig>> {
        self.agent.identity.as_ref().map(Option::as_ref)
    }

    fn execution_mode(&self) -> Option<&ExecutionMode> {
        self.agent.execution_mode.as_ref()
    }

    fn enabled(&self) -> Option<bool> {
        self.agent.enabled
    }

    fn permission(&self) -> Option<&AgentObjectPermission> {
        self.agent.object_permission.as_ref()
    }

    fn not_found(&self, agent_id: &str) -> String {
        format!("Agent with ID {agent_id} not found")
    }

    fn params_map(&self) -> Option<&BTreeMap<String, AgentJson>> {
        self.agent.litellm_params.as_ref()
    }

    fn name(&self) -> Option<String> {
        self.agent.agent_name.clone()
    }

    fn params(&self, row: Option<&StoredAgent>, context: &PlanContext<'_>) -> Option<String> {
        let incoming = self.params_map()?;
        let stored = stored_params(row);
        Some(params_text(Some(incoming), &stored, context))
    }

    fn card(&self, context: &PlanContext<'_>) -> ColumnResult {
        let Some(card) = self.agent.agent_card_params.as_ref() else {
            return Ok(None);
        };
        Ok(Some(card_text(Some(card), context)?))
    }

    fn headers(&self, context: &PlanContext<'_>) -> Option<String> {
        let headers = self.agent.static_headers.as_ref()?;
        Some(headers_text(headers.as_ref(), context.max_json_depth))
    }

    fn extra(&self) -> Option<Vec<String>> {
        self.agent
            .extra_headers
            .clone()
            .map(Option::unwrap_or_default)
    }

    fn kill_switch(&self, row: Option<&StoredAgent>) -> ColumnResult {
        let Some(switch) = self.agent.kill_switch.as_ref() else {
            return Ok(None);
        };
        let stored = row.and_then(|record| record.kill_switch_column.as_ref());
        Ok(Some(serialize_agent_kill_switch(switch.as_ref(), stored)?))
    }

    fn groups(&self) -> Option<Vec<String>> {
        let groups = self.agent.access_group_ids.as_ref()?;
        Some(deduplicated(&groups.clone().unwrap_or_default()))
    }

    fn limits(&self) -> Limits {
        [
            self.agent.tpm_limit,
            self.agent.rpm_limit,
            self.agent.session_tpm_limit,
            self.agent.session_rpm_limit,
        ]
    }

    fn created_by(&self, _actor: &str) -> Option<String> {
        None
    }

    fn created_at(&self, _context: &PlanContext<'_>) -> Option<NaiveDateTime> {
        None
    }
}
