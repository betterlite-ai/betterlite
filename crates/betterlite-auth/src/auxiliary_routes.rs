mod tests;
use std::collections::BTreeSet;

mod core;
mod mcp_agents;
mod public;
mod self_managed;

pub const OPENAI_ROUTE_NAMES: &[&str] = core::OPENAI_ROUTE_NAMES;
pub const MAPPED_PASS_THROUGH_ROUTES: &[&str] = core::MAPPED_PASS_THROUGH_ROUTES;
pub const PASSTHROUGH_ROUTES_WILDCARD: &[&str] = core::PASSTHROUGH_ROUTES_WILDCARD;
pub const LITELLM_NATIVE_ROUTES: &[&str] = core::LITELLM_NATIVE_ROUTES;
pub const MCP_INFERENCE_ROUTES: &[&str] = mcp_agents::MCP_INFERENCE_ROUTES;
pub const MCP_MANAGEMENT_ROUTES: &[&str] = mcp_agents::MCP_MANAGEMENT_ROUTES;
pub const MCP_ROUTES: &[&str] = mcp_agents::MCP_ROUTES;
pub const AGENT_INFERENCE_ROUTES: &[&str] = mcp_agents::AGENT_INFERENCE_ROUTES;
pub const AGENT_MANAGEMENT_ROUTES: &[&str] = mcp_agents::AGENT_MANAGEMENT_ROUTES;
pub const AGENT_ROUTES: &[&str] = mcp_agents::AGENT_ROUTES;
pub const GOOGLE_ROUTES: &[&str] = core::GOOGLE_ROUTES;
pub const APPLY_GUARDRAIL_ROUTES: &[&str] = core::APPLY_GUARDRAIL_ROUTES;
pub const MODEL_INFO_ROUTES: &[&str] = core::MODEL_INFO_ROUTES;
pub const INFO_ROUTES: &[&str] = core::INFO_ROUTES;
pub const UI_ROUTES: &[&str] = public::UI_ROUTES;
pub const COMPLIANCE_CHECK_ROUTES: &[&str] = public::COMPLIANCE_CHECK_ROUTES;
pub const SELF_MANAGED_ROUTES: &[&str] = self_managed::SELF_MANAGED_ROUTES;

#[must_use]
pub fn public_routes() -> BTreeSet<&'static str> {
    let mut routes = BTreeSet::new();
    for route in public::PUBLIC_ROUTES {
        routes.insert(*route);
    }
    routes
}
