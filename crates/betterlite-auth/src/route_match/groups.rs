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
    let routes = static_group(name)
        .map(<[_]>::to_vec)
        .or_else(|| dynamic_group(name))?;
    Some(routes.iter().map(|route| (*route).to_owned()).collect())
}

fn static_group(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "openai_route_names" => auxiliary_routes::OPENAI_ROUTE_NAMES,
        "openai_routes" => inference_routes::OPENAI_ROUTES,
        "mapped_pass_through_routes" => auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES,
        "passthrough_routes_wildcard" => auxiliary_routes::PASSTHROUGH_ROUTES_WILDCARD,
        "litellm_native_routes" => auxiliary_routes::LITELLM_NATIVE_ROUTES,
        "anthropic_routes" => inference_routes::ANTHROPIC_ROUTES,
        "mcp_inference_routes" => auxiliary_routes::MCP_INFERENCE_ROUTES,
        "mcp_management_routes" => auxiliary_routes::MCP_MANAGEMENT_ROUTES,
        "mcp_routes" => auxiliary_routes::MCP_ROUTES,
        "agent_inference_routes" => auxiliary_routes::AGENT_INFERENCE_ROUTES,
        "agent_management_routes" => auxiliary_routes::AGENT_MANAGEMENT_ROUTES,
        "agent_routes" => auxiliary_routes::AGENT_ROUTES,
        "google_routes" => auxiliary_routes::GOOGLE_ROUTES,
        "apply_guardrail_routes" => auxiliary_routes::APPLY_GUARDRAIL_ROUTES,
        "model_info_routes" => auxiliary_routes::MODEL_INFO_ROUTES,
        "info_routes" => auxiliary_routes::INFO_ROUTES,
        "master_key_only_routes" => management_routes::MASTER_KEY_ONLY_ROUTES,
        "key_management_routes" => management_routes::KEY_MANAGEMENT_ROUTES,
        "team_service_account_key_routes" => management_routes::TEAM_SERVICE_ACCOUNT_ROUTES,
        "spend_tracking_routes" => management_routes::SPEND_TRACKING_ROUTES,
        "global_spend_tracking_routes" => management_routes::GLOBAL_SPEND_TRACKING_ROUTES,
        "ui_routes" => auxiliary_routes::UI_ROUTES,
        "compliance_check_routes" => auxiliary_routes::COMPLIANCE_CHECK_ROUTES,
        "internal_user_view_only_routes" => management_routes::INTERNAL_USER_VIEW_ONLY_ROUTES,
        "self_managed_routes" => auxiliary_routes::SELF_MANAGED_ROUTES,
        "org_admin_only_routes" => management_routes::ORG_ADMIN_ONLY_ROUTES,
        _ => return None,
    })
}

fn dynamic_group(name: &str) -> Option<Vec<&str>> {
    let routes: Vec<&str> = match name {
        "llm_api_routes" => inference_routes::llm_api_routes(),
        "management_routes" => management_routes::management_routes(),
        "internal_user_routes" => management_routes::internal_user_routes(),
        "admin_viewer_routes" => management_routes::admin_viewer_routes(),
        "org_admin_allowed_routes" => management_routes::org_admin_allowed_routes(),
        "public_routes" => auxiliary_routes::public_routes().into_iter().collect(),
        _ => return None,
    };
    Some(routes)
}
