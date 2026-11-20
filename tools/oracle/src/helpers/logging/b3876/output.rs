use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use super::probe::{ProbeEvent, ProbeLog};

pub(super) fn events_value(log: &Arc<Mutex<ProbeLog>>) -> Value {
    let events: Vec<Value> = {
        let log = log.lock().unwrap_or_else(PoisonError::into_inner);
        log.events.iter().map(event_value).collect()
    };
    json!({ "events": events })
}

fn event_value(event: &ProbeEvent) -> Value {
    match event {
        ProbeEvent::Increment {
            metric,
            labels,
            amount,
        } => json!({
            "event": "increment",
            "metric": metric,
            "labels": labels,
            "amount": amount,
        }),
        ProbeEvent::Removal { metric, labels } => json!({
            "event": "removal",
            "metric": metric,
            "labels": labels,
        }),
    }
}
