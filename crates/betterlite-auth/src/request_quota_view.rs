use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use crate::agent_policy::AgentResponse;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// The key fields the v3 request-scope builders read, borrowed from one auth so the limiter crate can see them
#[derive(Debug)]
pub struct RequestQuotaView<'auth> {
    pub rpm_limit: Option<i64>,
    pub tpm_limit: Option<i64>,
    pub max_parallel_requests: Option<i64>,
    pub throttle_pct: Option<f64>,
    pub user_id: Option<&'auth str>,
    pub user_rpm_limit: Option<i64>,
    pub user_tpm_limit: Option<i64>,
    pub team_rpm_limit: Option<i64>,
    pub team_tpm_limit: Option<i64>,
    pub team_member_rpm_limit: Option<i64>,
    pub team_member_tpm_limit: Option<i64>,
    pub end_user_id: Option<&'auth str>,
    pub end_user_rpm_limit: Option<i64>,
    pub end_user_tpm_limit: Option<i64>,
    pub agent_id: Option<&'auth str>,
    pub invoked_agent_id: Option<&'auth str>,
    pub managed_agent_policy: Option<&'auth AgentResponse>,
    pub invoked_agent_policy: Option<&'auth AgentResponse>,
    pub rpm_limit_type: Option<&'auth str>,
    pub tpm_limit_type: Option<&'auth str>,
    pub source_team_rpm_limits: Option<&'auth BTreeMap<String, BTreeMap<String, i64>>>,
}

// The auth crate strips these from caller input, so the paths that own them attach them after auth as LiteLLM does
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ServerOnlyFields {
    pub managed_agent_policy: Option<AgentResponse>,
    pub invoked_agent_id: Option<String>,
    pub invoked_agent_policy: Option<AgentResponse>,
    pub source_team_rpm_limits: Option<BTreeMap<String, BTreeMap<String, i64>>>,
}

// The auth type keeps its one inherent impl in resolved_key, so the readers are a trait it implements here
pub trait RequestQuotaViewExt {
    #[must_use]
    fn request_quota_view(&self) -> RequestQuotaView<'_>;

    fn attach_server_only_fields(&mut self, fields: ServerOnlyFields);
}

fn metadata_text<'auth>(auth: &'auth UserAPIKeyAuth, name: &str) -> Option<&'auth str> {
    match auth.view.token.metadata.0.get(name) {
        Some(JsonData::String(text)) => Some(text.as_str()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}

impl RequestQuotaViewExt for UserAPIKeyAuth {
    fn request_quota_view(&self) -> RequestQuotaView<'_> {
        RequestQuotaView {
            rpm_limit: self.view.token.rpm_limit,
            tpm_limit: self.view.token.tpm_limit,
            max_parallel_requests: self.view.token.max_parallel_requests,
            throttle_pct: self.extra.budget_throttle_pct.and_then(|pct| pct.to_f64()),
            user_id: self.view.token.user_id.as_deref(),
            user_rpm_limit: self.extra.user_rpm_limit,
            user_tpm_limit: self.extra.user_tpm_limit,
            team_rpm_limit: self.view.team_rpm_limit,
            team_tpm_limit: self.view.team_tpm_limit,
            team_member_rpm_limit: self.view.team_member_rpm_limit,
            team_member_tpm_limit: self.view.team_member_tpm_limit,
            end_user_id: self.view.end_user_id.as_deref(),
            end_user_rpm_limit: self.view.end_user_rpm_limit,
            end_user_tpm_limit: self.view.end_user_tpm_limit,
            agent_id: self.view.token.agent_id.as_deref(),
            invoked_agent_id: self.extra.invoked_agent_id.as_deref(),
            managed_agent_policy: self.extra.managed_agent_policy.as_ref(),
            invoked_agent_policy: self.extra.invoked_agent_policy.as_ref(),
            rpm_limit_type: metadata_text(self, "rpm_limit_type"),
            tpm_limit_type: metadata_text(self, "tpm_limit_type"),
            source_team_rpm_limits: self.extra.mcp_source_team_rpm_limits.as_ref(),
        }
    }

    fn attach_server_only_fields(&mut self, fields: ServerOnlyFields) {
        self.extra.managed_agent_policy = fields.managed_agent_policy;
        self.extra.invoked_agent_id = fields.invoked_agent_id;
        self.extra.invoked_agent_policy = fields.invoked_agent_policy;
        self.extra.mcp_source_team_rpm_limits = fields.source_team_rpm_limits;
    }
}
