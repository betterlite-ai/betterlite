use crate::auth_request_models::RequestedModels;
use crate::route_match::classifier::is_llm_api_route;
use crate::route_match::templates::RouteValue;

use super::catalog::PricingCatalog;
use super::zero_cost::is_model_cost_zero;

// read-only discovery routes that incur no spend
pub const MODEL_DISCOVERY_ROUTES: [&str; 7] = [
    "/v1/models",
    "/models",
    "/model/info",
    "/v1/model/info",
    "/v2/model/info",
    "/model_group/info",
    "/utils/model_info",
];

// side-effectful routes an exhausted budget may not reach
pub const BUDGET_SIDE_EFFECT_ROUTES: [&str; 4] = [
    "/health",
    "/health/services",
    "/health/test_connection",
    "/auto_router/test_routing",
];

// route_skips_budget_checks: discovery and every non-LLM route, except the side-effectful ones
#[must_use]
pub fn route_skips_budget_checks(route: &str) -> bool {
    !BUDGET_SIDE_EFFECT_ROUTES.contains(&route)
        && (MODEL_DISCOVERY_ROUTES.contains(&route) || !is_llm_api_route(&RouteValue::Text(route)))
}

// request_skips_budget_checks: the route answers first, so a side-effect route can still be exempted by zero cost
#[must_use]
pub fn request_skips_budget_checks(
    route: &str,
    model: &RequestedModels,
    catalog: Option<&dyn PricingCatalog>,
) -> bool {
    if route_skips_budget_checks(route) {
        return true;
    }
    is_model_cost_zero(model, catalog)
}
