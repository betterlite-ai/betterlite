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

// One identity-scoped descriptor: its id, its two limits and an optional prefix for the counter value
struct IdentityScope<'request> {
    key: &'static str,
    id: Option<&'request str>,
    rpm_limit: Option<i64>,
    tpm_limit: Option<i64>,
    prefix: Option<&'request str>,
}

impl<'request> IdentityScope<'request> {
    const fn new(
        key: &'static str,
        id: Option<&'request str>,
        rpm_limit: Option<i64>,
        tpm_limit: Option<i64>,
        prefix: Option<&'request str>,
    ) -> Self {
        Self {
            key,
            id,
            rpm_limit,
            tpm_limit,
            prefix,
        }
    }
}

// LiteLLM's _create_rate_limit_descriptors: every key-owned descriptor, appended in the handler's order
pub fn create_rate_limit_descriptors(
    builder: &RequestScopeBuilder<'_>,
    inputs: &ScopeInputs<'_>,
) -> Result<Vec<RateLimitDescriptor>, KeyScopesError> {
    let view = inputs.auth.request_quota_view();
    let mut descriptors = Vec::new();
    push_key_limit(builder, &view, inputs, &mut descriptors);
    let identities = identity_scopes(&view, inputs);
    push_identity_limits(builder, &identities, &mut descriptors);
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
    push_tail_limits(builder, &view, inputs, model, &mut descriptors);
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
                throttled_limit(view.rpm_limit, view.throttle_pct),
                inputs.rpm_limit_type,
                inputs.model_has_failures,
            ),
            tokens_per_unit: get_enforced_limit(
                throttled_limit(view.tpm_limit, view.throttle_pct),
                inputs.tpm_limit_type,
                inputs.model_has_failures,
            ),
            max_parallel_requests: view.max_parallel_requests,
            window_size: Some(builder.window_size),
        }),
    });
}

// The four identity scopes LiteLLM builds, each with the id and limits its descriptor reads
fn identity_scopes<'auth>(
    view: &RequestQuotaView<'auth>,
    inputs: &ScopeInputs<'auth>,
) -> [IdentityScope<'auth>; 4] {
    let team_id = account_limits(inputs.auth).team_id;
    [
        IdentityScope::new(
            "user",
            view.user_id,
            view.user_rpm_limit,
            view.user_tpm_limit,
            None,
        ),
        IdentityScope::new(
            "team",
            team_id,
            view.team_rpm_limit,
            view.team_tpm_limit,
            None,
        ),
        IdentityScope::new(
            "team_member",
            view.user_id,
            view.team_member_rpm_limit,
            view.team_member_tpm_limit,
            Some(team_id.unwrap_or(NONE_TEXT)),
        ),
        IdentityScope::new(
            "end_user",
            view.end_user_id,
            view.end_user_rpm_limit,
            view.end_user_tpm_limit,
            None,
        ),
    ]
}

// The user, team, team_member and end_user descriptors, in LiteLLM's order
fn push_identity_limits(
    builder: &RequestScopeBuilder<'_>,
    identities: &[IdentityScope<'_>],
    out: &mut Vec<RateLimitDescriptor>,
) {
    for scope in identities {
        let Some(id) = scope.id.filter(|id| !id.is_empty()) else {
            continue;
        };
        if scope.rpm_limit.is_none() && scope.tpm_limit.is_none() {
            continue;
        }
        let value = scope
            .prefix
            .map_or_else(|| id.to_owned(), |prefix| format!("{prefix}:{id}"));
        out.push(rate_limit_descriptor(
            scope.key,
            value,
            scope.rpm_limit,
            scope.tpm_limit,
            builder.window_size,
        ));
    }
}

// The MCP pair, then the team model descriptor and the agents, in LiteLLM's order
fn push_tail_limits(
    builder: &RequestScopeBuilder<'_>,
    view: &RequestQuotaView<'_>,
    inputs: &ScopeInputs<'_>,
    model: Option<&str>,
    out: &mut Vec<RateLimitDescriptor>,
) {
    if inputs.call_type == Some(CALL_MCP_TOOL) && !inputs.data.contains_key("server_id") {
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
    builder.model.team_model_descriptor(inputs.auth, model, out);
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
