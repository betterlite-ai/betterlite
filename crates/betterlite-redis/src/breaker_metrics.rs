mod tests;

use std::hash::Hash;

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct StateLabel {
    state: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct FailureLabel {
    failure_class: String,
}

// a None registry means recording calls no-op instead of touching metrics that were never registered
pub struct BreakerMetrics {
    state_gauge: Option<Family<StateLabel, Gauge>>,
    transitions: Option<Family<StateLabel, Counter>>,
    failures: Option<Family<FailureLabel, Counter>>,
}

impl BreakerMetrics {
    #[must_use]
    pub fn new(target_registry: Option<&mut Registry>) -> Self {
        let Some(registry_handle) = target_registry else {
            return Self {
                state_gauge: None,
                transitions: None,
                failures: None,
            };
        };
        let state_gauge = Family::<StateLabel, Gauge>::default();
        registry_handle.register(
            "litellm_redis_circuit_breaker_state",
            "Number of Redis circuit breakers currently in each state",
            state_gauge.clone(),
        );
        let transitions = Family::<StateLabel, Counter>::default();
        registry_handle.register(
            "litellm_redis_circuit_breaker_transitions",
            "Redis circuit breaker state transitions",
            transitions.clone(),
        );
        let failures = Family::<FailureLabel, Counter>::default();
        registry_handle.register(
            "litellm_redis_circuit_breaker_failures",
            "Redis health failures counted by the circuit breaker",
            failures.clone(),
        );
        Self {
            state_gauge: Some(state_gauge),
            transitions: Some(transitions),
            failures: Some(failures),
        }
    }

    pub fn record_state_change(&self, old_state: Option<&str>, new_state: &str) {
        let Some(gauge) = &self.state_gauge else {
            return;
        };
        if let Some(previous_state) = old_state {
            gauge
                .get_or_create(&StateLabel {
                    state: previous_state.to_owned(),
                })
                .dec();
        }
        gauge
            .get_or_create(&StateLabel {
                state: new_state.to_owned(),
            })
            .inc();
    }

    pub fn record_transition(&self, state: &str) {
        let label = StateLabel {
            state: state.to_owned(),
        };
        Self::increment(self.transitions.as_ref(), &label);
    }

    pub fn record_failure(&self, failure_class: &str) {
        let label = FailureLabel {
            failure_class: failure_class.to_owned(),
        };
        Self::increment(self.failures.as_ref(), &label);
    }

    fn increment<L>(counters: Option<&Family<L, Counter>>, label: &L)
    where
        L: Clone + Eq + Hash + EncodeLabelSet,
    {
        let Some(family) = counters else {
            return;
        };
        family.get_or_create(label).inc();
    }
}
