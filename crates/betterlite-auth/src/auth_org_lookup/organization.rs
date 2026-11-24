use std::sync::Arc;

use betterlite_db::resource_rows::organization::OrganizationTable;

use super::error::{OrgFailure, OrgLookupError};
use super::ports::{OrgCache, OrgStore};

// get_org_object's dependencies: the Prisma client, the cache it reads first and the TTL it writes with
pub struct OrgLookup {
    pub store: Option<Arc<dyn OrgStore>>,
    pub cache: Arc<dyn OrgCache>,
    pub in_memory_ttl: f64,
}

// `not isinstance(org_id, str)`; a caller can hand over a key field that is not text
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrgId {
    Text(String),
    NotString,
}

pub async fn get_org_object(
    lookup: &OrgLookup,
    org_id: OrgId,
    include_budget: bool,
) -> Result<Option<OrganizationTable>, OrgLookupError> {
    let Some(store) = lookup.store.as_deref() else {
        return Err(OrgLookupError::NoDatabase);
    };
    let OrgId::Text(text_id) = org_id else {
        return Ok(None);
    };
    let key = cache_key(&text_id, include_budget);
    let cached = lookup
        .cache
        .read_org(&key, false)
        .await
        .map_err(|error| OrgLookupError::CacheRead(Box::new(error)))?;
    if let Some(org) = cached {
        return Ok(Some(org));
    }
    let found = store
        .find_org(&text_id, include_budget)
        .await
        .map_err(OrgFailure::Store)?;
    let Some(text) = found else {
        return Err(OrgLookupError::NotFound { org_id: text_id });
    };
    let org = serde_json::from_str::<OrganizationTable>(&text)
        .map_err(|source| OrgFailure::Row(Box::new(source)))?;
    lookup
        .cache
        .write_org(&key, &org, lookup.in_memory_ttl)
        .await
        .map_err(|error| OrgFailure::Cache(Box::new(error)))?;
    Ok(Some(org))
}

// include_budget_table picks a second key, so a budgetless hit never answers a caller that asked for one
fn cache_key(org_id: &str, include_budget: bool) -> String {
    if include_budget {
        return format!("org_id:{org_id}:with_budget");
    }
    format!("org_id:{org_id}")
}
