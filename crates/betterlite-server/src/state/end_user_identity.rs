use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_auth::auth_cache::{AuthCache, gateway_cache};
use betterlite_auth::auth_end_users::postgres::DatabaseEndUsers;
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_auth::auth_user_lookup::store::DatabaseUserStore;
use betterlite_auth::end_user_identity::existence::EndUserObjectExistence;
use betterlite_auth::end_user_identity::resolve::{DatabaseLookups, EndUserIdResolver};
use betterlite_auth::end_user_registry::EndUserRegistry;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::disk::Clock;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_config::sections::auth_cache::Settings as CacheSettings;
use betterlite_config::sections::auth_user_lookup::DefaultUserParams;
use betterlite_config::sections::end_user_registry::Settings as RegistrySettings;
use betterlite_config::sections::request_headers::Settings;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tokio_util::task::TaskTracker;

use crate::app::spend::cli_writer_store;
use crate::state::ServerState;
use crate::state::auth_resource_grants;
use crate::state::shared::Shared;

// betterlite-cache's MEMORY_MAX_SIZE_PER_ITEM_KIB, which is crate-private there
const CACHE_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;
// B736 keeps the user read throttle at DEFAULT_IN_MEMORY_TTL's 5-second default
const DB_CACHE_EXPIRY: i64 = 5;
// last_db_access_time's LimitedSizeOrderedDict bound
const CADENCE_MAX_SIZE: usize = 100;

type ProductionExistence = EndUserObjectExistence<
    DatabaseEndUsers,
    DatabaseEndUsers,
    AuthCache<DualCache<LocalMemoryOps>>,
>;

// what the production resolver needs from config beyond the two registry flags the slot exposes
struct ResolverSettings {
    registry: RegistrySettings,
    cache: CacheSettings,
    deadline_seconds: f64,
    defaults: Option<DefaultUserParams>,
}

enum Installed {
    Unbuilt,
    Failed,
    Ready(Arc<EndUserIdResolver>),
}

// the resolver and the settings it is built from, kept out of the slot's public flags
pub struct Validation {
    installed: Mutex<Installed>,
    settings: ResolverSettings,
}

// B10050's extraction settings, the B1218 validation flag and the resolver the proxy builds from them
pub struct Slot {
    pub database: bool,
    pub validate_end_user_id_in_db: bool,
    pub max_end_user_budget_id: Option<String>,
    pub request_headers: Settings,
    pub validation: Validation,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, shared: &Shared) -> Self {
        let registry = &config.sections.end_user_registry;
        Self {
            database: auth_resource_grants::Slot::new(config, shared).database,
            validate_end_user_id_in_db: registry.validate_end_user_id_in_db,
            max_end_user_budget_id: registry.max_end_user_budget_id.clone(),
            request_headers: config.sections.request_headers.clone(),
            validation: Validation {
                installed: Mutex::new(Installed::Unbuilt),
                settings: ResolverSettings {
                    registry: registry.clone(),
                    cache: config.sections.auth_cache,
                    deadline_seconds: config.sections.database_failures.deadline_seconds,
                    defaults: config
                        .sections
                        .auth_user_lookup
                        .default_internal_user_params
                        .clone(),
                },
            },
        }
    }

    // the first request that needs validation builds the resolver once, and every later request shares it
    #[must_use]
    pub fn resolver(&self, state: &Arc<ServerState>) -> Option<Arc<EndUserIdResolver>> {
        if !self.database || !self.validate_end_user_id_in_db {
            return None;
        }
        let mut installed = self
            .validation
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if matches!(&*installed, Installed::Unbuilt) {
            *installed = build_resolver(state, self).map_or(Installed::Failed, Installed::Ready);
        }
        match &*installed {
            Installed::Ready(resolver) => Some(Arc::clone(resolver)),
            Installed::Failed | Installed::Unbuilt => None,
        }
    }
}

// The production resolver: B733's cache, B739's row lookup, B736's user probe and B1218's registry.
fn build_resolver(state: &Arc<ServerState>, slot: &Slot) -> Option<Arc<EndUserIdResolver>> {
    let store = cli_writer_store(state)?;
    let cache = production_cache(&slot.validation.settings.cache)?;
    let end_users = Arc::new(DatabaseEndUsers {
        writer: Arc::clone(&store),
        reader: Arc::clone(&store),
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: slot.validation.settings.deadline_seconds,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    });
    let existence = ProductionExistence {
        rows: Some(Arc::<DatabaseEndUsers>::clone(&end_users)),
        budgets: Some(end_users),
        cache: Arc::clone(&cache),
        registry: Arc::new(EndUserRegistry::new(
            slot.validation.settings.registry.clone(),
        )),
        store: store.as_ref().clone(),
        max_end_user_budget_id: slot.max_end_user_budget_id.clone(),
    };
    let users = user_lookup(&store, &cache, &slot.validation.settings)?;
    Some(Arc::new(EndUserIdResolver {
        validate_end_user_id_in_db: slot.validate_end_user_id_in_db,
        max_end_user_budget_id: slot.max_end_user_budget_id.clone(),
        lookups: Some(DatabaseLookups {
            end_users: Arc::new(existence),
            users,
        }),
        cache,
    }))
}

fn production_cache(settings: &CacheSettings) -> Option<Arc<AuthCache<DualCache<LocalMemoryOps>>>> {
    let clock = wall_clock();
    let ordinary = LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, Arc::clone(&clock)).ok()?;
    let key_objects = LocalMemoryOps::new(CACHE_MAX_SIZE_PER_ITEM_KIB, clock).ok()?;
    Some(Arc::new(gateway_cache(
        Arc::new(ordinary),
        Arc::new(key_objects),
        None,
        DualCacheSettings {
            default_in_memory_ttl: Some(settings.gateway_memory_ttl),
            default_redis_ttl: settings.gateway_redis_ttl,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        settings.management_object_ttl,
    )))
}

fn user_lookup(
    store: &Arc<PostgresStore>,
    cache: &Arc<AuthCache<DualCache<LocalMemoryOps>>>,
    settings: &ResolverSettings,
) -> Option<Arc<UserLookup>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).ok()?;
    let lookup_cache = Arc::<AuthCache<DualCache<LocalMemoryOps>>>::clone(cache);
    Some(Arc::new(UserLookup {
        cache: lookup_cache,
        store: Some(Arc::new(DatabaseUserStore {
            reader: store.as_ref().clone(),
            writer: store.as_ref().clone(),
            availability: Arc::new(Availability::default()),
            default_deadline_seconds: settings.deadline_seconds,
            tracker: Arc::new(DbLookupStallTracker {
                clock: SystemClock,
                last_hit: Mutex::new(None),
            }),
            tasks: TaskTracker::new(),
        })),
        cadence: Arc::new(Cadence::new(CADENCE_MAX_SIZE, Arc::new(WallClock))),
        db_cache_expiry: DB_CACHE_EXPIRY,
        budget_reset: Arc::new(UnwiredBudgetReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: settings.defaults.clone(),
        redactor: Arc::new(redactor),
    }))
}

// the memory tiers expire by wall time, and every verdict write carries its own 300- or 60-second ttl
fn wall_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
    })
}

// B2317 owns get_budget_reset_time, and the existence probe never creates a user, so this port refuses
struct UnwiredBudgetReset;

impl BudgetResetCalculator for UnwiredBudgetReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("the budget reset time is not wired yet"))
    }
}
