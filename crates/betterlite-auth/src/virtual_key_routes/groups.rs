use crate::pass_through_registry::{PassThroughRegistry, PassThroughRouteType};
use crate::route_match::groups::get_actual_routes;

// a matching entry in one of these groups does not grant a registered auth=true pass-through route
pub const AUTH_ENFORCED_GROUPS: [&str; 2] = ["openai_routes", "llm_api_routes"];

const OPENAI_ROUTES: &str = "openai_routes";

#[must_use]
pub fn group_routes(name: &str, registry: &PassThroughRegistry) -> Vec<String> {
    let mut routes = get_actual_routes(&[name]);
    if name != OPENAI_ROUTES {
        return routes;
    }
    // LiteLLM appends every auth=true registration to openai_routes while it registers it
    for (_, metadata) in &registry.registrations {
        if metadata.auth != Some(true) {
            continue;
        }
        if !routes.contains(&metadata.path) {
            routes.push(metadata.path.clone());
        }
        if metadata.route_type != PassThroughRouteType::Subpath {
            continue;
        }
        let wildcard = format!("{}/*", metadata.path.trim_end_matches('/'));
        if !routes.contains(&wildcard) {
            routes.push(wildcard);
        }
    }
    routes
}
