use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;

pub const CLIENT_LIMIT_VARIABLE: &str = "MAX_LANGFUSE_INITIALIZED_CLIENTS";
pub const DEFAULT_CLIENT_LIMIT: usize = 50;

#[derive(Debug, Default)]
struct Counters {
    initialized: usize,
    pending: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LangfuseBudgetError {
    #[error("Max langfuse clients reached: {initialized} is greater than {limit}")]
    Exhausted { initialized: usize, limit: usize },
    #[error(transparent)]
    Limit(#[from] betterlite_env::EnvError),
}

// litellm.initialized_langfuse_clients plus the slots of the loggers being built right now
#[derive(Clone)]
pub struct LangfuseClientBudget {
    pub limit: usize,
    counters: Arc<Mutex<Counters>>,
}

impl LangfuseClientBudget {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            counters: Arc::default(),
        }
    }

    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Result<Self, LangfuseBudgetError> {
        let limit = match lookup.lookup(CLIENT_LIMIT_VARIABLE) {
            Some(raw) => betterlite_env::number::<usize>(CLIENT_LIMIT_VARIABLE, &raw)?
                .unwrap_or(DEFAULT_CLIENT_LIMIT),
            None => DEFAULT_CLIENT_LIMIT,
        };
        Ok(Self::new(limit))
    }

    // litellm.constants.MAX_LANGFUSE_INITIALIZED_CLIENTS, taken before the client is built
    pub fn reserve(&self) -> Result<LangfuseClientReservation, LangfuseBudgetError> {
        let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        if counters.initialized.saturating_add(counters.pending) >= self.limit {
            return Err(LangfuseBudgetError::Exhausted {
                initialized: counters.initialized,
                limit: self.limit,
            });
        }
        counters.pending = counters.pending.saturating_add(1);
        drop(counters);
        Ok(LangfuseClientReservation {
            budget: self.clone(),
            committed: false,
        })
    }

    pub fn release(&self) {
        let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        counters.initialized = counters.initialized.saturating_sub(1);
    }

    fn commit(&self) {
        let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        counters.pending = counters.pending.saturating_sub(1);
        counters.initialized = counters.initialized.saturating_add(1);
        let created = counters.initialized;
        drop(counters);
        tracing::debug!("Created langfuse client number {created}");
    }

    fn abandon(&self) {
        let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        counters.pending = counters.pending.saturating_sub(1);
    }
}

// a construction in flight holds a slot until it either lands in a logger or gives the slot back
pub struct LangfuseClientReservation {
    budget: LangfuseClientBudget,
    committed: bool,
}

impl LangfuseClientReservation {
    pub fn commit(mut self) {
        self.budget.commit();
        self.committed = true;
    }
}

impl Drop for LangfuseClientReservation {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.budget.abandon();
    }
}
