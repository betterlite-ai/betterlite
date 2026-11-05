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
    pub call_types: Vec<String>,
    pub routes: Vec<RouteEntry>,
}

impl RouteCatalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, RouteMetadataError> {
        if bytes.len() > MAX_CATALOG_BYTES {
            return Err(RouteMetadataError::CatalogTooLong);
        }
        let catalog: Self = serde_json::from_slice(bytes)?;
        for name in &catalog.call_types {
            serde_json::from_value::<CallType>(serde_json::Value::String(name.clone()))?;
        }
        for entry in &catalog.routes {
            validate_entry(entry, &catalog.call_types)?;
        }
        Ok(catalog)
    }
}

fn validate_entry(entry: &RouteEntry, declared: &[String]) -> Result<(), RouteMetadataError> {
    if entry.call_types.is_empty() {
        return Err(RouteMetadataError::EmptyRouteTypes);
    }
    for call_type in &entry.call_types {
        if !is_declared(*call_type, declared)? {
            return Err(RouteMetadataError::UndeclaredCallType);
        }
    }
    Ok(())
}

fn is_declared(call_type: CallType, declared: &[String]) -> Result<bool, RouteMetadataError> {
    let value = serde_json::to_value(call_type)?;
    Ok(value
        .as_str()
        .is_some_and(|name| declared.iter().any(|known| known == name)))
}

mod tests;
