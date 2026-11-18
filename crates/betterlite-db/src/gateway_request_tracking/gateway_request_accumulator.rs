use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;

use super::GATEWAY_REQUEST_DATE_FORMAT;
use super::GatewayRequestClock;
use super::SystemClock;
use super::fold_gateway_counts::fold_counts;

mod tests;

pub struct GatewayRequestAccumulator {
    pub counts: Mutex<GatewayRequestSnapshot>,
    pub clock: Arc<dyn GatewayRequestClock>,
}

impl GatewayRequestAccumulator {
    pub fn record(&self, category: &str, route: &str, status_code: u16) {
        let date = self
            .clock
            .utc_now()
            .format(GATEWAY_REQUEST_DATE_FORMAT)
            .to_string();
        let key = GatewayRequestKey {
            date,
            category: category.to_owned(),
            route: route.to_owned(),
        };
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let current = counts.get(&key).copied().unwrap_or_default();
        let _replaced = counts.insert(key, current.plus((200..300).contains(&status_code)));
    }

    #[must_use]
    pub fn drain(&self) -> GatewayRequestSnapshot {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts
            .extract_if(.., |_key, _value| true)
            .collect::<GatewayRequestSnapshot>()
    }

    pub fn restore(&self, snapshot: &GatewayRequestSnapshot) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let current = mem::take(&mut *counts);
        let merged = current
            .into_iter()
            .chain(snapshot.iter().map(|(key, value)| (key.clone(), *value)));
        *counts = fold_counts(merged);
    }
}

impl Default for GatewayRequestAccumulator {
    fn default() -> Self {
        Self {
            counts: Mutex::new(BTreeMap::new()),
            clock: Arc::new(SystemClock),
        }
    }
}
