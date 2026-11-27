use std::sync::Arc;

use betterlite_auth::auth_policy_support::extract::{ToolNameExtractors, ToolRouteCallTypes};
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::failure_hook::lift::call_type_name;

// B207's pinned route catalog answers the same route table LiteLLM's get_call_types_for_route does
#[derive(Debug)]
struct CatalogRoutes {
    catalog: RouteCatalog,
}

impl ToolRouteCallTypes for CatalogRoutes {
    fn call_types_for_route(&self, route: &str) -> Vec<String> {
        let Ok(Some(types)) = self.catalog.call_types_for(route) else {
            return Vec::new();
        };
        types
            .iter()
            .filter_map(|call_type| call_type_name(*call_type))
            .collect()
    }
}

pub(crate) fn extractors() -> Result<ToolNameExtractors, Box<HelperError>> {
    let catalog = RouteCatalog::parse(CATALOG).or_value_error()?;
    let routes = Arc::new(CatalogRoutes { catalog });
    Ok(ToolNameExtractors::builtin(routes))
}
