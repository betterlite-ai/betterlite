use std::sync::Arc;

use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_cache::disk::Clock;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio_util::task::TaskTracker;

use crate::management_admins::adapters::LookupUserReads;
use crate::management_admins::ports::{AdminUserReads, UserReadError};

use super::support::{FakeCache, user};

struct UnusedReset;

impl BudgetResetCalculator for UnusedReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("the cache-hit test never resets a budget"))
    }
}

fn lookup_state(cache: Arc<dyn UserCache>) -> UserLookup {
    UserLookup {
        cache,
        store: None,
        cadence: Arc::new(Cadence::new(1024_usize, Arc::new(WallClock))),
        db_cache_expiry: 60_i64,
        budget_reset: Arc::new(UnusedReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")),
    }
}

#[tokio::test]
async fn user_object_reads_the_given_cache() {
    let populated: Arc<dyn UserCache> = Arc::new(FakeCache::new(60.0_f64));
    populated
        .write("u1", &user("u1", &[]), 60.0_f64)
        .await
        .expect("the fixture user caches");
    let clock: Clock = Arc::new(|| 0.0_f64);
    let reads = LookupUserReads {
        lookup: Arc::new(lookup_state(Arc::clone(&populated))),
        clock,
        management_ttl: 60.0_f64,
    };
    let request = UserLookupRequest {
        user_id: Some("u1"),
        sso_user_id: None,
        user_email: None,
        user_id_upsert: false,
        check_db_only: None,
    };
    let found = reads
        .user_object(request, Arc::clone(&populated))
        .await
        .expect("a cache hit needs no store");
    assert_eq!(found.map(|row| row.user_id), Some("u1".to_owned()));
    let empty: Arc<dyn UserCache> = Arc::new(FakeCache::new(60.0_f64));
    let missing = reads.user_object(request, empty).await;
    let Err(UserReadError::Lookup(error)) = missing else {
        panic!("a cache miss without a store is a lookup failure");
    };
    assert!(matches!(error, UserLookupError::NoDatabase));
}
