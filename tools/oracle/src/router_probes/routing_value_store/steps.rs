use betterlite_router::routing_value_store::{
    RouteIncrement, RouteKey, RouteValue, RoutingValueStore, StoreRead,
};

use super::parser::{compare_output, read_value};
use super::{Call, Entry, ReadResult, StepOutput, StepResult};
use crate::router_probes::error::OperationError;

pub(super) fn execute_set(call: &Call<'_>, value: &RouteValue) -> StepResult {
    if let Some(route_key) = &call.key {
        call.store.set(route_key, value, call.ttl)?;
    }
    Ok(StepOutput::Set)
}

pub(super) fn execute_set_pipeline(call: &Call<'_>, entries: &[Entry]) -> StepResult {
    let parsed = entries.iter().filter_map(entry_key).collect::<Vec<_>>();
    call.store.set_pipeline(&parsed, call.ttl)?;
    Ok(StepOutput::SetPipeline)
}

fn entry_key(entry: &Entry) -> Option<(RouteKey, RouteValue)> {
    RouteKey::parse(&entry.key)
        .ok()
        .map(|key| (key, entry.value.clone()))
}

pub(super) fn execute_set_add(call: &Call<'_>, values: &[String]) -> StepResult {
    let added = match &call.key {
        Some(route_key) => call.store.set_add(route_key, values, call.ttl)?,
        None => values.to_vec(),
    };
    Ok(StepOutput::SetAdd { values: added })
}

pub(super) fn read_one(store: &RoutingValueStore, raw: &str) -> ReadResult {
    let Ok(route_key) = RouteKey::parse(raw) else {
        return Ok(None);
    };
    let read = store.get(&route_key)?;
    Ok(read_value(&read))
}

pub(super) fn execute_batch_get(store: &RoutingValueStore, keys: &[String]) -> StepResult {
    let values = keys
        .iter()
        .map(|key| read_one(store, key))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StepOutput::BatchGet { values })
}

pub(super) fn execute_increment(call: &Call<'_>, by: &RouteIncrement) -> StepResult {
    let route_key = call
        .key
        .as_ref()
        .ok_or_else(|| missing_key("an increment"))?;
    let value = call.store.increment(route_key, by, call.ttl)?;
    Ok(StepOutput::Increment { value })
}

pub(super) fn execute_compare_and_set(
    call: &Call<'_>,
    expected: Option<&RouteValue>,
    value: &RouteValue,
) -> StepResult {
    let route_key = call
        .key
        .as_ref()
        .ok_or_else(|| missing_key("a compare and set"))?;
    let wanted = expected.map_or(StoreRead::Missing, |current| {
        StoreRead::Value(current.clone())
    });
    let outcome = call
        .store
        .compare_and_set(route_key, &wanted, value, call.ttl)?;
    Ok(compare_output(&outcome))
}

fn missing_key(operation: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: format!("{operation} needs a route key within the key limit"),
    }
}
