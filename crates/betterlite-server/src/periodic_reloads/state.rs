use std::sync::{Mutex, PoisonError};

use betterlite_models::source::{CatalogProvenance, CatalogSnapshot};
use chrono::{DateTime, Utc};

struct ReloadState {
    cost_map_applied_revision: u32,
    cost_map_loaded_at: DateTime<Utc>,
    last_beta_headers_reload: Option<DateTime<Utc>>,
}

pub struct PeriodicReloads {
    state: Mutex<ReloadState>,
}

impl PeriodicReloads {
    #[must_use]
    pub const fn new(provenance: &CatalogProvenance, now: DateTime<Utc>) -> Self {
        Self {
            state: Mutex::new(ReloadState {
                // a fresh pod starts unapplied, so its first poll adopts a schedule row already above zero
                cost_map_applied_revision: 0,
                cost_map_loaded_at: provenance.loaded_at.unwrap_or(now),
                last_beta_headers_reload: None,
            }),
        }
    }

    #[must_use]
    pub fn cost_map_applied_revision(&self) -> u32 {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cost_map_applied_revision
    }

    pub fn set_cost_map_applied_revision(&self, revision: u32) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cost_map_applied_revision = revision;
    }

    #[must_use]
    pub fn cost_map_loaded_at(&self) -> DateTime<Utc> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cost_map_loaded_at
    }

    #[must_use]
    pub fn last_beta_headers_reload(&self) -> Option<DateTime<Utc>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_beta_headers_reload
    }

    pub fn set_last_beta_headers_reload(&self, at: DateTime<Utc>) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_beta_headers_reload = Some(at);
    }

    #[must_use]
    pub fn swap_in_model_cost_map(&self, snapshot: &CatalogSnapshot, at: DateTime<Utc>) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cost_map_loaded_at = at;
        snapshot.catalog.adopted_size
    }
}

mod tests;
