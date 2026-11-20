pub mod cache;
pub mod error;
pub mod index;
pub mod load;
pub mod rows;

#[path = "membership_load/tests/postgres.rs"]
mod postgres;
mod tests;

use std::sync::Arc;

use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tokio::sync::{oneshot, watch};
use tokio_util::task::TaskTracker;

use self::cache::{CachedMembership, CachedPayload, MembershipCache, classify_cached};
use self::error::MembershipLoadError;
use self::index::{MembershipIndex, SharedEntry};
use self::load::{LoadRequest, MembershipLoad, run_load, shared_result};
use self::rows::TeamMembershipRows;
use crate::auth_cache::keys::CacheKey;
use crate::auth_invalidation::registry::InflightLoad;

// auth_checks.py's get_team_membership: the local read, the shared load waiters join, and the DB gate.
pub struct MembershipLookup {
    pub cache: Arc<dyn MembershipCache>,
    // None is the source's prisma_client=None; a miss with no joinable load has no database to read.
    pub rows: Option<Arc<dyn TeamMembershipRows>>,
    pub index: Arc<dyn MembershipIndex>,
    pub tasks: TaskTracker,
}

impl MembershipLookup {
    pub async fn get_team_membership(
        &self,
        user_id: Option<&str>,
        team_id: Option<&str>,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        let (Some(user_id), Some(team_id)) = (user_id, team_id) else {
            return Ok(None);
        };
        let key = CacheKey::TeamMembershipReservation { user_id, team_id }.render();
        let cached = self
            .cache
            .read(&key, true)
            .await
            .map_err(MembershipLoadError::CacheRead)?;
        // The first read is local only: a local member or negative sentinel returns before any DB check.
        match classify_cached(CachedPayload::from(cached))? {
            CachedMembership::Membership(membership) => Ok(Some(*membership)),
            CachedMembership::Absent => Ok(None),
            CachedMembership::Miss => self.load_or_share(user_id, team_id, key).await,
        }
    }

    async fn load_or_share(
        &self,
        user_id: &str,
        team_id: &str,
        key: String,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        // Only a task joins; an entry of any other shape falls through to the database gate.
        if let Some(SharedEntry::Load(load)) = self.index.entry(&key) {
            return shared_result(await_completion(load.completion).await?);
        }
        let Some(rows) = self.rows.as_ref() else {
            return Err(MembershipLoadError::NoDatabase);
        };
        let (start, started) = oneshot::channel();
        let (completion, receiver) = watch::channel(None::<MembershipLoad>);
        let load_task = self.tasks.spawn(run_load(LoadRequest {
            start: started,
            completion,
            index: Arc::clone(&self.index),
            key: key.clone(),
            user_id: user_id.to_owned(),
            team_id: team_id.to_owned(),
            rows: Arc::clone(rows),
            cache: Arc::clone(&self.cache),
        }));
        let watching = receiver.clone();
        self.index.install(
            &key,
            InflightLoad {
                id: load_task.id(),
                completion: receiver,
            },
        );
        let Ok(()) = start.send(()) else {
            return Err(MembershipLoadError::LoadLost);
        };
        shared_result(await_completion(watching).await?)
    }
}

// Every waiter reads the one shared result; a failure is shared as one Arc, as one exception is.
async fn await_completion(
    mut completion: watch::Receiver<Option<MembershipLoad>>,
) -> Result<MembershipLoad, MembershipLoadError> {
    loop {
        {
            let current = completion.borrow_and_update();
            if let Some(load) = current.as_ref() {
                return Ok(load.clone());
            }
        }
        if completion.changed().await.is_err() {
            return Err(MembershipLoadError::LoadLost);
        }
    }
}
