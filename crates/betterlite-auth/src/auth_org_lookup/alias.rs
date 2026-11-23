use betterlite_db::resource_rows::organization::OrganizationTable;

use super::error::{OrgFailure, OrgLookupError};
use super::organization::OrgLookup;
use super::ports::OrgStore;

// The three outcomes inside get_org_object_by_alias's second try block, each with its own answer.
enum AliasError {
    NotFound,
    Ambiguous,
    Failed(OrgFailure),
}

pub async fn get_org_object_by_alias(
    lookup: &OrgLookup,
    org_alias: &str,
) -> Result<OrganizationTable, OrgLookupError> {
    let Some(store) = lookup.store.as_deref() else {
        return Err(OrgLookupError::NoDatabase);
    };
    // the cache read sits before the try block, so its failure is never relabeled as the 500
    let key = format!("org_alias:{org_alias}");
    let cached = lookup
        .cache
        .read_org(&key, false)
        .await
        .map_err(|error| OrgLookupError::CacheRead(Box::new(error)))?;
    if let Some(org) = cached {
        return Ok(org);
    }
    match find_single(lookup, store, org_alias, &key).await {
        Ok(org) => Ok(org),
        Err(AliasError::NotFound) => Err(OrgLookupError::AliasNotFound {
            org_alias: org_alias.to_owned(),
        }),
        Err(AliasError::Ambiguous) => Err(OrgLookupError::AliasAmbiguous {
            org_alias: org_alias.to_owned(),
        }),
        Err(AliasError::Failed(cause)) => {
            tracing::error!(
                org_alias,
                error = %cause,
                "Error looking up organization by alias"
            );
            Err(OrgLookupError::AliasFailure {
                org_alias: org_alias.to_owned(),
                cause: Box::new(cause),
            })
        }
    }
}

async fn find_single(
    lookup: &OrgLookup,
    store: &dyn OrgStore,
    org_alias: &str,
    alias_key: &str,
) -> Result<OrganizationTable, AliasError> {
    let rows = store
        .find_by_alias(org_alias)
        .await
        .map_err(|error| AliasError::Failed(OrgFailure::Store(error)))?;
    match rows.len() {
        0 => Err(AliasError::NotFound),
        1 => {
            let Some(text) = rows.into_iter().next() else {
                return Err(AliasError::NotFound);
            };
            let org = serde_json::from_str::<OrganizationTable>(&text)
                .map_err(|source| AliasError::Failed(OrgFailure::Row(Box::new(source))))?;
            cache_alias(lookup, alias_key, &org).await?;
            Ok(org)
        }
        _ => Err(AliasError::Ambiguous),
    }
}

// both writes share DEFAULT_IN_MEMORY_TTL and there is no rollback when the second one fails
async fn cache_alias(
    lookup: &OrgLookup,
    alias_key: &str,
    org: &OrganizationTable,
) -> Result<(), AliasError> {
    let org_id = org.organization_id.as_deref().unwrap_or("None");
    let id_key = format!("org_id:{org_id}");
    for key in [alias_key, id_key.as_str()] {
        lookup
            .cache
            .write_org(key, org, lookup.in_memory_ttl)
            .await
            .map_err(|error| AliasError::Failed(OrgFailure::Cache(Box::new(error))))?;
    }
    Ok(())
}
