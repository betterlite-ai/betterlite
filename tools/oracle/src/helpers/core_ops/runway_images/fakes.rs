use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::runway::tasks::poll::PollClock;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};

pub(super) struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<u64>>,
}

impl Clock {
    pub(super) fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::default(),
        }
    }

    pub(super) fn pauses(&self) -> Vec<u64> {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration.as_secs());
        ready(())
    }
}

pub(super) struct EmptyLookup;

impl PriceLookup for EmptyLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}
