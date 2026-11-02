use super::templates::{
    RouteValue, check_route_access, route_matches_pattern, route_matches_wildcard_pattern,
};
use crate::{auxiliary_routes, inference_routes, management_routes};
use regex::Regex;

#[must_use]
pub fn is_llm_api_route(route: &RouteValue<'_>) -> bool {
    let RouteValue::Text(path) = route else {
        return false;
    };
    matches_exact_tables(path)
        || matches_template_tables(path)
        || is_azure_openai_route(&RouteValue::Text(path))
        || matches_mapped_prefix(path)
}

fn matches_exact_tables(route: &str) -> bool {
    inference_routes::OPENAI_ROUTES.contains(&route)
        || inference_routes::ANTHROPIC_ROUTES.contains(&route)
        || auxiliary_routes::GOOGLE_ROUTES.contains(&route)
        || check_route_access(route, auxiliary_routes::MCP_INFERENCE_ROUTES)
        || check_route_access(route, auxiliary_routes::AGENT_INFERENCE_ROUTES)
        || auxiliary_routes::LITELLM_NATIVE_ROUTES.contains(&route)
}

fn matches_template_tables(route: &str) -> bool {
    let templated = |pattern: &&str| pattern.contains('{');
    let wildcarded = |pattern: &&str| matches!(pattern.chars().last(), Some('*'));
    let hits_pattern = |pattern: &&str| route_matches_pattern(&RouteValue::Text(route), pattern);
    let hits_wildcard = |pattern: &&str| route_matches_wildcard_pattern(route, pattern);

    inference_routes::OPENAI_ROUTES.iter().any(|pattern| {
        (templated(pattern) && hits_pattern(pattern))
            || (wildcarded(pattern) && hits_wildcard(pattern))
    }) || auxiliary_routes::GOOGLE_ROUTES
        .iter()
        .chain(inference_routes::ANTHROPIC_ROUTES.iter())
        .any(|pattern| templated(pattern) && hits_pattern(pattern))
}

fn matches_mapped_prefix(route: &str) -> bool {
    auxiliary_routes::MAPPED_PASS_THROUGH_ROUTES
        .iter()
        .any(|prefix| route == *prefix || route.starts_with(&format!("{prefix}/")))
}

#[must_use]
pub fn is_mcp_discovery_route(route: &str, method: Option<&str>) -> bool {
    let Some(verb) = method else {
        return false;
    };
    if verb.to_uppercase() != "GET" {
        return false;
    }
    if route == "/v1/mcp/server" {
        return true;
    }
    let Some(remainder) = route.strip_prefix("/v1/mcp/server/") else {
        return false;
    };
    !remainder.is_empty() && !remainder.contains('/')
}

#[must_use]
pub fn is_management_route(route: &str) -> bool {
    if route.is_empty() {
        return false;
    }
    check_route_access(route, &management_routes::management_routes())
}

#[must_use]
pub fn is_info_route(route: &str) -> bool {
    if route.is_empty() {
        return false;
    }
    auxiliary_routes::INFO_ROUTES.contains(&route)
}

#[must_use]
pub fn is_azure_openai_route(route: &RouteValue<'_>) -> bool {
    let RouteValue::Text(text) = route else {
        return false;
    };
    let Ok(deployment_pattern) = Regex::new("^/openai/deployments/[^/]+/[^/]+/chat/completions$")
    else {
        return false;
    };
    let Ok(engine_pattern) = Regex::new("^/engines/[^/]+/chat/completions$") else {
        return false;
    };
    deployment_pattern.is_match(text) || engine_pattern.is_match(text)
}
