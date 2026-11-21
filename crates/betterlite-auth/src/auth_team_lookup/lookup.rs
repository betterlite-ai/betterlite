use std::sync::Arc;

use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use tracing::debug;

use super::ports::{TeamCache, TeamProvisioner, TeamStore};
use crate::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, ObjectPermissionSource, PermissionError, PermissionStores,
    get_object_permission,
};
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_user_lookup::cadence::Cadence;

// LiteLLM's prisma_client: absent means every lookup fails before it reads the cache
pub struct TeamDatabase {
    pub store: Arc<dyn TeamStore>,
    pub permission_reader: Arc<dyn ObjectPermissionStore>,
    pub permission_writer: Arc<dyn ObjectPermissionStore>,
}

pub struct TeamLookup {
    pub cache: Arc<dyn TeamCache>,
    pub permission_cache: Arc<dyn ObjectPermissionCache>,
    pub database: Option<TeamDatabase>,
    pub provisioner: Option<Arc<dyn TeamProvisioner>>,
    // auth_checks.py's last_db_access_time, the same index get_user_object keeps
    pub cadence: Arc<Cadence>,
    // DEFAULT_IN_MEMORY_TTL: the alias cache TTL, and db_cache_expiry for the read throttle
    pub in_memory_ttl: i32,
}

// check_cache_only and check_db_only; check_db_only wins when a caller sets both
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeamSource {
    CacheThenDatabase,
    CacheOnly,
    Writer,
}

#[derive(Clone, Copy, Debug)]
pub struct TeamRequest<'request> {
    pub team_id: &'request str,
    pub source: TeamSource,
    pub team_id_upsert: bool,
}

// An absent object_permission loads through B735; a writer read propagates its failure, any other logs it.
pub(super) async fn attach_permission(
    lookup: &TeamLookup,
    database: &TeamDatabase,
    team: &mut LiteLLMTeamTableCachedObj,
    use_writer: bool,
) -> Result<(), Box<PermissionError>> {
    let fields = &team.team.fields;
    let Some(permission_id) = fields.object_permission_id.clone() else {
        return Ok(());
    };
    if fields.object_permission.is_some() {
        return Ok(());
    }
    let source = ObjectPermissionSource {
        cache: lookup.permission_cache.as_ref(),
        stores: Some(PermissionStores {
            normal: database.permission_reader.as_ref(),
            writer: database.permission_writer.as_ref(),
        }),
    };
    match get_object_permission(&source, &permission_id, use_writer).await {
        Ok(permission) => team.team.fields.object_permission = permission,
        Err(error) if use_writer => return Err(Box::new(error)),
        Err(error) => {
            let team_id = &team.team.fields.base.team_id;
            debug!(
                team_id = team_id.as_str(),
                object_permission_id = permission_id.as_str(),
                error = %error,
                "Failed to load object_permission for team {team_id} with object_permission_id={permission_id}: {error}"
            );
        }
    }
    Ok(())
}
