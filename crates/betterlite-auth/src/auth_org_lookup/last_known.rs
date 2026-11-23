use betterlite_db::database_availability::policy::{
    AllowRequestsOnOutage, allows_requests_during_outage,
};
use betterlite_db::resource_rows::organization::OrganizationTable;

use super::error::{OrgFailure, OrgLookupError, is_outage};
use super::organization::{OrgId, OrgLookup, get_org_object};

// get_org_object_for_request: absence is "no restriction", an outage keeps the org ceiling from cache
pub async fn get_org_object_for_request(
    lookup: &OrgLookup,
    org_id: &str,
    policy: &AllowRequestsOnOutage,
) -> Result<Option<OrganizationTable>, OrgLookupError> {
    let last_known_key = format!("org_id:{org_id}:with_budget:last_known");
    let found = match get_org_object(lookup, OrgId::Text(org_id.to_owned()), true).await {
        Ok(org) => org,
        Err(OrgLookupError::NotFound { .. }) => return Ok(None),
        Err(error) if is_outage(&error) => {
            return outage_fallback(lookup, &last_known_key, policy, error).await;
        }
        Err(error) => {
            tracing::debug!(
                error = %error,
                "org lookup failed, continuing without org limits"
            );
            return Ok(None);
        }
    };
    let Some(org) = found else {
        return Ok(None);
    };
    keep_last_known_org(lookup, &last_known_key, &org).await?;
    Ok(Some(org))
}

// the last known copy is read with local_only cleared, so a replica holding it still answers
async fn outage_fallback(
    lookup: &OrgLookup,
    last_known_key: &str,
    policy: &AllowRequestsOnOutage,
    error: OrgLookupError,
) -> Result<Option<OrganizationTable>, OrgLookupError> {
    let stale = lookup
        .cache
        .read_org(last_known_key, false)
        .await
        .map_err(|cause| OrgFailure::Cache(Box::new(cause)))?;
    if let Some(org) = stale {
        return Ok(Some(org));
    }
    let allowed = allows_requests_during_outage(policy).map_err(OrgFailure::Policy)?;
    if allowed {
        return Ok(None);
    }
    Err(error)
}

// the local copy is only written when none is held, and it lives for the management object TTL
async fn keep_last_known_org(
    lookup: &OrgLookup,
    last_known_key: &str,
    org: &OrganizationTable,
) -> Result<(), OrgLookupError> {
    let held = lookup
        .cache
        .read_org(last_known_key, true)
        .await
        .map_err(|error| OrgFailure::Cache(Box::new(error)))?;
    if held.is_some() {
        return Ok(());
    }
    let ttl = lookup.cache.management_ttl();
    lookup
        .cache
        .write_org(last_known_key, org, ttl)
        .await
        .map_err(|error| OrgFailure::Cache(Box::new(error)))
}
