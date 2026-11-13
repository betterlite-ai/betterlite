use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::{Baggage, BaggageExt as _};

// litellm.integrations.otel.plumbing.context.set_request_baggage
#[must_use]
pub fn set_request_baggage(
    values: &BTreeMap<String, String>,
    context: Option<&Context>,
) -> Context {
    let base = context.cloned().unwrap_or_else(Context::current);
    let mut baggage = Baggage::new();
    for (key, (value, metadata)) in base.baggage() {
        baggage.insert_with_metadata(key.to_string(), value.clone(), metadata.clone());
    }
    for (key, value) in values {
        baggage.insert(key.clone(), value.clone());
    }
    base.with_baggage(baggage)
}

#[path = "baggage_context/tests.rs"]
mod tests;
