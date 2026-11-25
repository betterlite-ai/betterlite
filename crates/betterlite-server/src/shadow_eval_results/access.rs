use crate::plugin_routes::routes::Identity;
use crate::shadow_eval_lifecycle::access::require_admin;
use crate::shadow_eval_validation::access::AdminRole;
use crate::shadow_result_aggregation::repository::ShadowResultsRepository;

use super::error::ShadowResultsRouteError;
use super::store::ShadowLegReads;

// _require_admin_viewer then the database check, in the order the listing endpoint declares them
pub fn require_viewer<'store, Store>(
    identity: &Identity,
    action: &str,
    store: Option<&'store Store>,
) -> Result<&'store Store, ShadowResultsRouteError>
where
    Store: ShadowResultsRepository + ShadowLegReads + ?Sized,
{
    require_admin(identity, action, AdminRole::Viewer)?;
    store.ok_or(ShadowResultsRouteError::DatabaseUnavailable)
}
