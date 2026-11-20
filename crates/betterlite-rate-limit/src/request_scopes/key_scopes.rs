use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::budget_primitives::throttle::throttled_limit;
use betterlite_auth::request_quota_view::RequestQuotaView;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::request_destination::RequestData;

use super::RequestScopeBuilder;
use super::body_text;
use super::limit_policy::get_enforced_limit;
use crate::identity_descriptors::agent_resolution::resolved_agent_id;
use crate::identity_descriptors::descriptor::rate_limit_descriptor;
use crate::identity_descriptors::mcp_per_key::add_mcp_per_key_descriptor;
use crate::identity_descriptors::mcp_per_team::add_mcp_per_team_descriptor;
use crate::identity_descriptors::tag_per_key::add_tag_per_key_descriptor;
use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};
use crate::model_descriptors::ModelDescriptorsError;

mod tests;

const CALL_MCP_TOOL: &str = "call_mcp_tool";
// LiteLLM formats a missing team id into the team_member value with an f-string, so the text is "None"
const NONE_TEXT: &str = "None";

#[derive(Debug, thiserror::Error)]
pub enum KeyScopesError {
    #[error(transparent)]
    Model(#[from] ModelDescriptorsError),
}

// The six inputs _create_rate_limit_descriptors takes, with the limit types read by the caller
pub struct ScopeInputs<'request> {
    pub auth: &'request UserAPIKeyAuth,
    pub data: &'request RequestData,
    pub rpm_limit_type: Option<&'request str>,
    pub tpm_limit_type: Option<&'request str>,
    pub model_has_failures: bool,
    pub call_type: Option<&'request str>,
}

// LiteLLM's _create_rate_limit_descriptors: every key-owned descriptor, appended in the handler's order
pub fn create_rate_limit_descriptors(
    builder: &RequestScopeBuilder<'_>,
    inputs: ScopeInputs<'_>,
) -> Result<Vec<RateLimitDescriptor>, KeyScopesError> {
    let view = inputs.auth.request_quota_view();
    let mut descriptors = Vec::new();
    push_key_limit(builder, &view, &inputs, &mut descriptors);
    push_user_limit(builder, &view, &mut descriptors);
    push_team_limit(builder, &view, &inputs, &mut descriptors);
    push_team_member_limit(builder, &view, &inputs, &mut descriptors);
    push_end_user_limit(builder, &view, &mut descriptors);
    let model = body_text(inputs.data, "model");
    builder
        .model
        .key_model_descriptor(inputs.auth, model, &mut descriptors)?;
    add_tag_per_key_descriptor(
        inputs.auth,
        inputs.data,
        builder.window_size,
        &mut descriptors,
    );
    push_mcp_limits(builder, &view, &inputs, &mut descriptors);
    builder
        .model
        .team_model_descriptor(inputs.auth, model, &mut descriptors);
    push_agent_limits(builder, &view, &inputs, &mut descriptors);
    Ok(descriptors)
}

fn push_key_limit(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(api_key) = account_limits(inputs.auth)
        .hashed_api_key
        .filter(|key| !key.is_empty())
    else {
        return;
    };
    if view.rpm_limit.is_none() && view.tpm_limit.is_none() && view.max_parallel_requests.is_none()
    {
        return;
    }
    out.push(RateLimitDescriptor {
        key: "api_key".to_owned(),
        value: api_key.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: get_enforced_limit(
                throttled_limit(view.rpm_limit, view.budget_throttle_pct),
                inputs.rpm_limit_type,
                inputs.model_has_failures,
            ),
            tokens_per_unit: get_enforced_limit(
                throttled_limit(view.tpm_limit, view.budget_throttle_pct),
                inputs.tpm_limit_type,
                inputs.model_has_failures,
            ),
            max_parallel_requests: view.max_parallel_requests,
            window_size: Some(builder.window_size),
        }),
    });
}

fn push_user_limit(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(user_id) = view.user_id.filter(|id| !id.is_empty()) else {
        return;
    };
    if view.user_rpm_limit.is_none() && view.user_tpm_limit.is_none() {
        return;
    }
    out.push(rate_limit_descriptor(
        "user",
        user_id.to_owned(),
        view.user_rpm_limit,
        view.user_tpm_limit,
        builder.window_size,
    ));
}

fn push_team_limit(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(team_id) = account_limits(inputs.auth)
        .team_id
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    if view.team_rpm_limit.is_none() && view.team_tpm_limit.is_none() {
        return;
    }
    out.push(rate_limit_descriptor(
        "team",
        team_id.to_owned(),
        view.team_rpm_limit,
        view.team_tpm_limit,
        builder.window_size,
    ));
}

fn push_team_member_limit(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(user_id) = view.user_id.filter(|id| !id.is_empty()) else {
        return;
    };
    if view.team_member_rpm_limit.is_none() && view.team_member_tpm_limit.is_none() {
        return;
    }
    let team_id = account_limits(inputs.auth).team_id.unwrap_or(NONE_TEXT);
    out.push(rate_limit_descriptor(
        "team_member",
        format!("{team_id}:{user_id}"),
        view.team_member_rpm_limit,
        view.team_member_tpm_limit,
        builder.window_size,
    ));
}

fn push_end_user_limit(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let Some(end_user_id) = view.end_user_id.filter(|id| !id.is_empty()) else {
        return;
    };
    if view.end_user_rpm_limit.is_none() && view.end_user_tpm_limit.is_none() {
        return;
    }
    out.push(rate_limit_descriptor(
        "end_user",
        end_user_id.to_owned(),
        view.end_user_rpm_limit,
        view.end_user_tpm_limit,
        builder.window_size,
    ));
}

fn push_mcp_limits(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    if inputs.call_type != Some(CALL_MCP_TOOL) || inputs.data.contains_key("server_id") {
        return;
    }
    let server = body_text(inputs.data, "mcp_server_name");
    add_mcp_per_key_descriptor(inputs.auth, server, builder.window_size, out);
    add_mcp_per_team_descriptor(
        inputs.auth,
        server,
        view.source_team_rpm_limits,
        builder.window_size,
        out,
    );
}

fn push_agent_limits(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    let resolved = resolved_agent_id(view.agent_id, inputs.data);
    let mut handled: Vec<&str> = Vec::new();
    for agent_id in [resolved, view.invoked_agent_id].into_iter().flatten() {
        if handled.contains(&agent_id) {
            continue;
        }
        handled.push(agent_id);
        let policy = if Some(agent_id) == view.agent_id {
            view.managed_agent_policy
        } else {
            view.invoked_agent_policy
        };
        out.extend(
            builder
                .agents
                .create_agent_descriptors(agent_id, inputs.data, policy),
        );
    }
}
