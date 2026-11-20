use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tokio::sync::{oneshot, watch};
use tokio::task::try_id;

use super::cache::{CachedMembership, CachedPayload, MembershipCache, classify_cached};
use super::error::MembershipLoadError;
use super::index::MembershipIndex;
use super::rows::TeamMembershipRows;
use crate::auth_read::membership::{SharedMembershipLoad, membership_from_shared_load};

// What the shared load leaves for every waiter.
#[derive(Clone)]
pub enum MembershipLoad {
    Present(Box<LiteLLMTeamMembership>),
    // A non-membership payload, so B2315's projection can answer None without decoding it again.
    Foreign(Box<CacheValue>),
    Absent,
    Failed(Arc<MembershipLoadError>),
}

// The caller installs the entry and then opens the start gate, so completion cleanup always runs last.
pub(super) struct LoadRequest {
    pub start: oneshot::Receiver<()>,
    pub completion: watch::Sender<Option<MembershipLoad>>,
    pub index: Arc<dyn MembershipIndex>,
    pub key: String,
    pub user_id: String,
    pub team_id: String,
    pub rows: Arc<dyn TeamMembershipRows>,
    pub cache: Arc<dyn MembershipCache>,
}

pub(super) async fn run_load(request: LoadRequest) {
    let load = if request.start.await.is_ok() {
        load_membership(
            &request.user_id,
            &request.team_id,
            &request.key,
            request.rows.as_ref(),
            request.cache.as_ref(),
        )
        .await
    } else {
        // The caller left before opening the gate, so no waiter exists and only the entry needs removing.
        MembershipLoad::Failed(Arc::new(MembershipLoadError::LoadLost))
    };
    request.completion.send_replace(Some(load));
    if let Some(id) = try_id() {
        request.index.remove_if(&request.key, id);
    }
}

// The source's _load_team_membership_on_cache_miss: a second read that also reaches Redis, then SQL.
async fn load_membership(
    user_id: &str,
    team_id: &str,
    key: &str,
    rows: &dyn TeamMembershipRows,
    cache: &dyn MembershipCache,
) -> MembershipLoad {
    match resolved(user_id, team_id, key, rows, cache).await {
        Ok(Some(membership)) => MembershipLoad::Present(Box::new(membership)),
        Ok(None) => MembershipLoad::Absent,
        Err(error) => MembershipLoad::Failed(Arc::new(error)),
    }
}

async fn resolved(
    user_id: &str,
    team_id: &str,
    key: &str,
    rows: &dyn TeamMembershipRows,
    cache: &dyn MembershipCache,
) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
    let cached = cache
        .read(key, false)
        .await
        .map_err(MembershipLoadError::CacheRead)?;
    match classify_cached(CachedPayload::from(cached))? {
        CachedMembership::Membership(membership) => return Ok(Some(*membership)),
        CachedMembership::Absent => return Ok(None),
        CachedMembership::Miss => {}
    }
    let found = rows.find_membership(user_id, team_id).await?;
    match &found {
        // A present row is written without ttl, so the selected cache backend's default applies.
        Some(membership) => cache
            .write_membership(key, membership, None)
            .await
            .map_err(MembershipLoadError::CacheWrite)?,
        None => cache
            .write_absent(key, Some(cache.management_ttl()))
            .await
            .map_err(MembershipLoadError::CacheWrite)?,
    }
    Ok(found)
}

// B2315's projection keeps an already typed membership and rejects anything else without decoding it.
pub(super) fn shared_result(
    load: MembershipLoad,
) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
    match load {
        MembershipLoad::Present(membership) => Ok(membership_from_shared_load(
            SharedMembershipLoad::Membership(membership),
        )),
        MembershipLoad::Foreign(payload) => Ok(membership_from_shared_load(
            SharedMembershipLoad::Foreign(payload),
        )),
        MembershipLoad::Absent => Ok(None),
        MembershipLoad::Failed(error) => Err(MembershipLoadError::Shared(error)),
    }
}
