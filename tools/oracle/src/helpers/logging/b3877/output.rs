use std::sync::PoisonError;

use serde_json::{Value, json};

use super::probe::{ProbeCounters, ProbeEvent};

pub(super) fn events_value(counters: &ProbeCounters) -> Value {
    let events: Vec<Value> = {
        let log = counters.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.events.iter().map(event_value).collect()
    };
    json!({ "events": events })
}

fn event_value(recorded: &ProbeEvent) -> Value {
    match recorded {
        ProbeEvent::Update {
            event,
            metric,
            labels,
            amount,
        } => json!({
            "event": event,
            "metric": metric,
            "labels": labels,
            "amount": amount,
        }),
    }
}
