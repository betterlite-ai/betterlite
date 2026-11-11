use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage, usage_sum};
use indexmap::IndexMap;

use super::error::RealtimeCostError;
use super::usage::aggregate_mapping;
use super::{Event, RealtimeBilling};

// The validated probe of _ResponsesWsEvent: unrelated event types are validated before the billable filter.
pub struct BillableEvent<'event> {
    pub event: &'event Event,
    pub usage: &'event IndexMap<String, Json>,
    pub service_tier: Option<&'event str>,
}

struct ValidatedEvent<'event> {
    event_type: &'event str,
    usage: Option<&'event IndexMap<String, Json>>,
    service_tier: Option<&'event str>,
}

fn validated_type(event: &Event) -> Result<&str, RealtimeCostError> {
    match event.get("type") {
        None => Ok(""),
        Some(Json::Text(text)) => Ok(text.as_str()),
        Some(_) => Err(RealtimeCostError::Shape {
            field: "type",
            expected: "a string",
        }),
    }
}

fn optional_mapping<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'fields IndexMap<String, Json>>, RealtimeCostError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(found)) => Ok(Some(found)),
        Some(_) => Err(RealtimeCostError::Shape {
            field: name,
            expected: "an object or null",
        }),
    }
}

fn validate_event(event: &Event) -> Result<ValidatedEvent<'_>, RealtimeCostError> {
    let event_type = validated_type(event)?;
    let Some(fields) = optional_mapping(event, "response")? else {
        return Ok(ValidatedEvent {
            event_type,
            usage: None,
            service_tier: None,
        });
    };
    let usage = optional_mapping(fields, "usage")?;
    let service_tier = match fields.get("service_tier") {
        None | Some(Json::Null) => None,
        Some(Json::Text(tier)) => Some(tier.as_str()),
        Some(_) => {
            return Err(RealtimeCostError::Shape {
                field: "service_tier",
                expected: "a string or null",
            });
        }
    };
    Ok(ValidatedEvent {
        event_type,
        usage,
        service_tier,
    })
}

pub fn billable_events(results: &[Event]) -> Result<Vec<BillableEvent<'_>>, RealtimeCostError> {
    let mut billable = Vec::new();
    for event in results {
        let validated = validate_event(event)?;
        if !matches!(
            validated.event_type,
            "response.completed" | "response.incomplete"
        ) {
            continue;
        }
        let Some(usage) = validated.usage else {
            continue;
        };
        billable.push(BillableEvent {
            event,
            usage,
            service_tier: validated.service_tier,
        });
    }
    Ok(billable)
}

pub fn collect_usage(results: &[Event]) -> Result<Vec<Usage>, RealtimeCostError> {
    billable_events(results)?
        .into_iter()
        .map(|billable| aggregate_mapping(billable.usage))
        .collect()
}

// Ports partition_results_by_service_tier: first-seen tier order, None distinct from an empty tier.
pub fn partition_by_service_tier(
    results: &[Event],
) -> Result<IndexMap<Option<String>, Vec<Event>>, RealtimeCostError> {
    let mut partition: IndexMap<Option<String>, Vec<Event>> = IndexMap::new();
    for billable in billable_events(results)? {
        let service_tier = billable.service_tier.map(str::to_owned);
        partition
            .entry(service_tier)
            .or_default()
            .push(billable.event.clone());
    }
    Ok(partition)
}

pub fn split_by_service_tier(
    record: &RealtimeBilling,
) -> Result<Option<Vec<RealtimeBilling>>, RealtimeCostError> {
    let partition = partition_by_service_tier(&record.results)?;
    if partition.len() <= 1 {
        return Ok(None);
    }
    let mut parts = Vec::with_capacity(partition.len());
    for (service_tier, events) in partition {
        let usages = collect_usage(&events)?;
        let usage = usage_sum(&usages).map_err(RealtimeCostError::from)?;
        parts.push(RealtimeBilling {
            results: events,
            usage,
            service_tier,
        });
    }
    Ok(Some(parts))
}
