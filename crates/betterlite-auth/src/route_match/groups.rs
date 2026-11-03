use crate::{auxiliary_routes, inference_routes, management_routes};

#[must_use]
pub fn get_actual_routes(allowed_routes: &[&str]) -> Vec<String> {
    let mut actual_routes = Vec::new();
    for route_name in allowed_routes {
        match expand_group(route_name) {
            Some(group) => actual_routes.extend(group),
            None => actual_routes.push((*route_name).to_owned()),
        }
    }
    actual_routes
}

fn expand_group(name: &str) -> Option<Vec<String>> {
    let routes: Vec<&str> = match name {
        "openai_route_names" => auxiliary_routes::OPENAI_ROUTE_NAMES.to_vec(),
        "openai_routes" => inference_routes::OPENAI_ROUTES.to_vec(),
        "mapped_pass_through_routes" => auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES.to_vec(),
        "passthrough_routes_wildcard" => auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD.to_vec(),
        "litellm_native_routes" => auxiliary_routes::LITELLM_NATIVE_ROUTES.to_vec(),
        "anthropic_routes" => inference_routes::ANTHROPIC_ROUTES.to_vec(),
        "mcp_inference_routes" => auxiliary_routes::MCP_INFERENCE_ROUTES.to_vec(),
        "mcp_management_routes" => auxiliary_routes::MCP_MANAGEMENT_ROUTES.to_vec(),
        "mcp_routes" => auxiliary_routes::MCP_ROUTES.to_vec(),
        "agent_inference_routes" => auxiliary_routes::AGENT_INFERENCE_ROUTES.to_vec(),
        "agent_management_routes" => auxiliary_routes::AGENT_MANAGEMENT_ROUTES.to_vec(),
        "agent_routes" => auxiliary_routes::AGENT_ROUTES.to_vec(),
        "google_routes" => auxiliary_routes::GOOGLE_ROUTES.to_vec(),
        "apply_guardrail_routes" => auxiliary_routes::APPLY_GUARDRAIL_ROUTES.to_vec(),
        "model_info_routes" => auxiliary_routes::MODEL_INFO_ROUTES.to_vec(),
        "info_routes" => auxiliary_routes::INFO_ROUTES.to_vec(),
        "master_key_only_routes" => management_routes::MASTER_KEY_ONLY_ROUTES.to_vec(),
        "key_management_routes" => management_routes::KEY_MANAGEMENT_ROUTES.to_vec(),
        "team_service_account_key_routes" => {
            management_routes::TEAM_SERVICE_ACCOUNT_ROUTES.to_vec()
        }
        "spend_tracking_routes" => management_routes::SPEND_TRACKING_ROUTES.to_vec(),
        "global_spend_tracking_routes" => management_routes::GLOBAL_SPEND_TRACKING_ROUTES.to_vec(),
        "ui_routes" => auxiliary_routes::UI_ROUTES.to_vec(),
        "compliance_check_routes" => auxiliary_routes::COMPLIANCE_CHECK_ROUTES.to_vec(),
        "internal_user_view_only_routes" => {
            management_routes::INTERNAL_USER_VIEW_ONLY_ROUTES.to_vec()
        }
        "self_managed_routes" => auxiliary_routes::SELF_MANAGED_ROUTES.to_vec(),
        "org_admin_only_routes" => management_routes::ORG_ADMIN_ONLY_ROUTES.to_vec(),
        "llm_api_routes" => inference_routes::llm_api_routes(),
        "management_routes" => management_routes::management_routes(),
        "internal_user_routes" => management_routes::internal_user_routes(),
        "admin_viewer_routes" => management_routes::admin_viewer_routes(),
        "org_admin_allowed_routes" => management_routes::org_admin_allowed_routes(),
        "public_routes" => auxiliary_routes::public_routes().into_iter().collect(),
        _ => return None,
    };
    Some(routes.into_iter().map(str::to_owned).collect())
}
