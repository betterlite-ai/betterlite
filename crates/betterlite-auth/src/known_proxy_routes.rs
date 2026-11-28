use std::collections::BTreeSet;

use crate::auxiliary_routes;
use crate::inference_routes;
use crate::management_routes;

mod tests;

// the member tables LiteLLMRoutes lists directly and that are consts; the two inference statics and its six builders are extended below
const MEMBER_TABLES: [&[&str]; 24] = [
    auxiliary_routes::OPENAI_ROUTE_NAMES,
    auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES,
    auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD,
    auxiliary_routes::LITELLM_NATIVE_ROUTES,
    auxiliary_routes::MCP_INFERENCE_ROUTES,
    auxiliary_routes::MCP_MANAGEMENT_ROUTES,
    auxiliary_routes::MCP_ROUTES,
    auxiliary_routes::AGENT_INFERENCE_ROUTES,
    auxiliary_routes::AGENT_MANAGEMENT_ROUTES,
    auxiliary_routes::AGENT_ROUTES,
    auxiliary_routes::GOOGLE_ROUTES,
    auxiliary_routes::APPLY_GUARDRAIL_ROUTES,
    auxiliary_routes::MODEL_INFO_ROUTES,
    auxiliary_routes::INFO_ROUTES,
    auxiliary_routes::UI_ROUTES,
    auxiliary_routes::COMPLIANCE_CHECK_ROUTES,
    auxiliary_routes::SELF_MANAGED_ROUTES,
    management_routes::MASTER_KEY_ONLY_ROUTES,
    management_routes::KEY_MANAGEMENT_ROUTES,
    management_routes::TEAM_SERVICE_ACCOUNT_ROUTES,
    management_routes::SPEND_TRACKING_ROUTES,
    management_routes::GLOBAL_SPEND_TRACKING_ROUTES,
    management_routes::INTERNAL_USER_VIEW_ONLY_ROUTES,
    management_routes::ORG_ADMIN_ONLY_ROUTES,
];

// LiteLLM's KNOWN_PROXY_ROUTES: every route that starts with "/" in every LiteLLMRoutes member
#[must_use]
pub fn known_proxy_routes() -> BTreeSet<&'static str> {
    let mut members = MEMBER_TABLES.concat();
    members.extend_from_slice(inference_routes::OPENAI_ROUTES);
    members.extend_from_slice(inference_routes::ANTHROPIC_ROUTES);
    members.extend(auxiliary_routes::public_routes());
    members.extend(inference_routes::llm_api_routes());
    members.extend(management_routes::management_routes());
    members.extend(management_routes::internal_user_routes());
    members.extend(management_routes::admin_viewer_routes());
    members.extend(management_routes::org_admin_allowed_routes());
    members
        .into_iter()
        .filter(|route| route.starts_with('/'))
        .collect()
}

// allowlisted(path, KNOWN_PROXY_ROUTES): a route template such as "/key/{key_id}/reset_spend" matches only that exact text
#[must_use]
pub fn known_proxy_route(path: &str) -> Option<&'static str> {
    // every route starts with "/" because known_proxy_routes filters on it, so other text cannot match
    if !path.starts_with('/') {
        return None;
    }
    known_proxy_routes().get(path).copied()
}
