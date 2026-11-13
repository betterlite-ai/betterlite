use serde::{Deserialize, Serialize};

use super::{CallType, RouteMetadataError};

pub const CATALOG: &[u8] = include_bytes!("../../fixtures/route_metadata.json");

const MAX_CATALOG_BYTES: usize = 0x0002_0000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RouteEntry {
    pub route: String,
    pub call_types: Vec<CallType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RouteCatalog {
    pub source: String,
    pub call_types: Vec<CallType>,
    pub routes: Vec<RouteEntry>,
}

impl RouteCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, RouteMetadataError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(RouteMetadataError::CatalogTooLong);
        }
        let catalog: Self = serde_json::from_slice(bytes)?;
        for entry in &catalog.routes {
            validate_entry(entry, &catalog.call_types)?;
        }
        Ok(catalog)
    }

    pub fn call_types_for(&self, route: &str) -> Result<Option<&[CallType]>, RouteMetadataError> {
        if route.len() > 4096 {
            return Err(RouteMetadataError::RouteTooLong);
        }
        if let Some(exact) = self.routes.iter().find(|entry| entry.route == route) {
            return Ok(Some(&exact.call_types));
        }
        for entry in &self.routes {
            if super::pattern::pattern_matches(route, &entry.route)? {
                return Ok(Some(&entry.call_types));
            }
        }
        Ok(None)
    }

    #[must_use]
    pub fn routes_for(&self, call_type: CallType) -> Vec<&str> {
        self.routes
            .iter()
            .filter(|entry| entry.call_types.contains(&call_type))
            .map(|entry| entry.route.as_str())
            .collect()
    }
}

fn validate_entry(entry: &RouteEntry, declared: &[CallType]) -> Result<(), RouteMetadataError> {
    if entry.call_types.is_empty() {
        return Err(RouteMetadataError::EmptyRouteTypes);
    }
    for call_type in &entry.call_types {
        if !declared.contains(call_type) {
            return Err(RouteMetadataError::UndeclaredCallType);
        }
    }
    Ok(())
}

mod tests;
