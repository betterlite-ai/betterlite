use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::router_observations::keys::ROUTER_COUNTER_TTL;
use crate::routing_state::counter::RouterCounters;
use crate::routing_state::delta::DeltaStore;
use crate::routing_value_store::{RouteKey, RouteValue, RoutingValueStore, StoreRead};

use super::keys::window_keys;

// One deployment's usage inside one minute: the tpm and rpm values the pinned map holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowUsage {
    pub tokens: i64,
    pub requests: i64,
}

impl WindowUsage {
    // a deployment with no stored counter in this minute, the pinned map's missing-key read
    pub const ZERO: Self = Self {
        tokens: 0,
        requests: 0,
    };
}

// The cost windows a success observation writes and a selection reads.
#[async_trait]
pub trait CostWindows: Send + Sync {
    fn usage(&self, model_group: &str, deployment_id: &str, minute: &str) -> WindowUsage;

    async fn record(&self, model_group: &str, deployment_id: &str, minute: &str, tokens: i64);
}

// B304's router counters hold each window as a spend value that expires with the minute.
pub struct WindowCounters<S: DeltaStore + 'static> {
    pub counters: Arc<RouterCounters<S>>,
    pub values: Arc<RoutingValueStore>,
}

#[async_trait]
impl<S: DeltaStore + 'static> CostWindows for WindowCounters<S> {
    fn usage(&self, model_group: &str, deployment_id: &str, minute: &str) -> WindowUsage {
        let keys = window_keys(model_group, deployment_id, minute);
        WindowUsage {
            tokens: amount(&self.values, &keys.tokens),
            requests: amount(&self.values, &keys.requests),
        }
    }

    async fn record(&self, model_group: &str, deployment_id: &str, minute: &str, tokens: i64) {
        let keys = window_keys(model_group, deployment_id, minute);
        for (key, value) in [(&keys.tokens, tokens), (&keys.requests, 1)] {
            if let Err(error) = self
                .counters
                .increment_window_value(key, Decimal::from(value), ROUTER_COUNTER_TTL)
                .await
            {
                tracing::warn!(%error, key = key.as_str(), "the cost window increment failed");
            }
        }
    }
}

// A route carries no window store, so every deployment it selects from starts the minute unused.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ColdWindows;

#[async_trait]
impl CostWindows for ColdWindows {
    fn usage(&self, _model_group: &str, _deployment_id: &str, _minute: &str) -> WindowUsage {
        WindowUsage::ZERO
    }

    async fn record(&self, _model_group: &str, _deployment_id: &str, _minute: &str, _tokens: i64) {}
}

fn amount(values: &RoutingValueStore, key: &str) -> i64 {
    let Ok(route_key) = RouteKey::parse(key) else {
        tracing::warn!(key, "a cost window key is longer than the key limit");
        return 0;
    };
    match values.get(&route_key) {
        Ok(StoreRead::Value(RouteValue::Spend(value))) => value.to_i64().unwrap_or_default(),
        Ok(StoreRead::Value(RouteValue::Counter(value))) => value,
        Ok(
            StoreRead::Value(
                RouteValue::Pin(_) | RouteValue::SchedulerTuple(_) | RouteValue::Members(_),
            )
            | StoreRead::Missing,
        ) => 0,
        Err(error) => {
            tracing::warn!(%error, key, "a cost window value could not be read");
            0
        }
    }
}
