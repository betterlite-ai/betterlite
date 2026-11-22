use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_config::sections::settings::settings_store::Resolution;
use betterlite_config::sections::settings::{ConfigOwnedKeyError, JsonValue, SettingsStore};

use crate::job_scheduler::{JobFn, JobSpec, ProxyScheduler, SchedulerError, SchedulerState};
use crate::scheduled_job_stagger::{NextRunTime, Trigger};

use super::SPEND_LOG_CLEANUP_JOB_ID;

// The slice of the scheduler the cleanup jobs use; the oracle drives a recording implementation.
pub trait CleanupScheduler {
    fn scheduler_state(&self) -> SchedulerState;
    fn has_job(&self, id: &str) -> bool;
    fn insert_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError>;
    fn remove_existing_job(&mut self, id: &str) -> Result<(), SchedulerError>;
}

impl CleanupScheduler for ProxyScheduler {
    fn scheduler_state(&self) -> SchedulerState {
        Self::state(self)
    }

    fn has_job(&self, id: &str) -> bool {
        self.get_job(id).is_some()
    }

    fn insert_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError> {
        Self::add_job(self, spec)
    }

    fn remove_existing_job(&mut self, id: &str) -> Result<(), SchedulerError> {
        Self::remove_job(self, id)
    }
}

// The general settings view both hooks write through, so the database sync can hand in its store.
pub trait GeneralSettingsView {
    fn get(&self, key: &str) -> Option<&JsonValue>;
    fn owned_by_config(&self, key: &str) -> bool;
    fn set(&mut self, key: &str, value: JsonValue) -> Result<(), ConfigOwnedKeyError>;
    fn remove_key(&mut self, key: &str) -> Result<(), ConfigOwnedKeyError>;
}

impl GeneralSettingsView for SettingsStore {
    fn get(&self, key: &str) -> Option<&JsonValue> {
        Self::get(self, key)
    }

    fn owned_by_config(&self, key: &str) -> bool {
        matches!(self.resolution(key), Resolution::Config(_))
    }

    fn set(&mut self, key: &str, value: JsonValue) -> Result<(), ConfigOwnedKeyError> {
        self.insert(key, value)
    }

    fn remove_key(&mut self, key: &str) -> Result<(), ConfigOwnedKeyError> {
        self.remove(key).map(|_previous| ())
    }
}

pub trait CacheMaxSize {
    fn resize(&self, max_size: Option<usize>);
}

impl<T: AuthCacheTier> CacheMaxSize for AuthCache<T> {
    fn resize(&self, max_size: Option<usize>) {
        Self::update_in_memory_max_size(self, max_size);
    }
}

#[must_use]
pub(super) fn cleanup_job_spec(trigger: Box<dyn Trigger>, job: JobFn) -> JobSpec {
    JobSpec {
        id: SPEND_LOG_CLEANUP_JOB_ID.to_owned(),
        trigger,
        run: job,
        replace_existing: true,
        next_run_time: NextRunTime::Unset,
        misfire_grace_seconds: None,
        max_instances: None,
    }
}
