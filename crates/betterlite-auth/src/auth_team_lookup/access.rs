use betterlite_db::resource_rows::access_group::AccessGroupTable;
use tracing::error;

use super::error::{TeamFailure, TeamLookupError};
use super::lookup::TeamLookup;

// auth_checks.py get_access_object: check_db_only skips the cache and reads the writer
pub async fn get_access_object(
    lookup: &TeamLookup,
    access_group_id: &str,
    check_db_only: bool,
) -> Result<AccessGroupTable, TeamLookupError> {
    let Some(database) = lookup.database.as_ref() else {
        return Err(TeamLookupError::NoDatabase);
    };
    let key = format!("access_group_id:{access_group_id}");
    if !check_db_only {
        let cached = lookup
            .cache
            .read_access_group(&key)
            .await
            .map_err(|error| TeamLookupError::CacheRead(Box::new(error)))?;
        if let Some(table) = cached {
            return Ok(table);
        }
    }
    let failed = |cause: TeamFailure| access_failure(access_group_id, check_db_only, cause);
    let found = database
        .store
        .find_access_group(access_group_id, check_db_only)
        .await
        .map_err(|error| failed(TeamFailure::Store(Box::new(error))))?;
    let Some(row) = found else {
        return Err(TeamLookupError::AccessGroupNotFound {
            access_group_id: access_group_id.to_owned(),
        });
    };
    let wire = serde_json::to_value(&row).map_err(|error| failed(TeamFailure::AccessRow(error)))?;
    let table = serde_json::from_value::<AccessGroupTable>(wire)
        .map_err(|error| failed(TeamFailure::AccessRow(error)))?;
    lookup
        .cache
        .cache_access_group(access_group_id, &table)
        .await
        .map_err(|error| failed(TeamFailure::Cache(Box::new(error))))?;
    Ok(table)
}

// any failure but the absent row: an ERROR log, then a 503 policy outage on the writer or the ordinary 404
fn access_failure(
    access_group_id: &str,
    check_db_only: bool,
    cause: TeamFailure,
) -> TeamLookupError {
    error!(access_group_id, error = %cause, "Error getting access group for access_group_id: {access_group_id}");
    let id = access_group_id.to_owned();
    let boxed = Box::new(cause);
    if check_db_only {
        return TeamLookupError::AccessPolicyUnavailable {
            access_group_id: id,
            cause: boxed,
        };
    }
    TeamLookupError::AccessGroupFailure {
        access_group_id: id,
        cause: boxed,
    }
}
