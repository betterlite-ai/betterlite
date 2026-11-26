use std::sync::Arc;

use betterlite_auth::auth_policy_support::extract::{ToolNameExtractors, ToolRouteCallTypes};
use betterlite_config::types::Config;
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};

use crate::failure_hook::lift::call_type_name;
use crate::state::shared::Shared;

// get_call_types_for_route over B207's pinned catalog, so the stage learns which routes can carry tools
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

#[derive(Clone, Debug)]
pub struct Slot {
    pub extractors: Option<Arc<ToolNameExtractors>>,
}

impl Slot {
    #[must_use]
    pub fn new(_config: &Config, _shared: &Shared) -> Self {
        Self {
            extractors: pinned_extractors(),
        }
    }
}

// a catalog that no longer parses leaves the tool allowlist unenforced, so it says so once at startup
fn pinned_extractors() -> Option<Arc<ToolNameExtractors>> {
    let catalog = match RouteCatalog::parse(CATALOG) {
        Ok(catalog) => catalog,
        Err(error) => {
            tracing::error!(error = %error, "the pinned route metadata catalog does not parse");
            return None;
        }
    };
    let routes = Arc::new(CatalogRoutes { catalog });
    Some(Arc::new(ToolNameExtractors::builtin(routes)))
}
