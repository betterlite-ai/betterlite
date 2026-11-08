use betterlite_router::routing_value_store::codec::parse_route_value;
use betterlite_router::routing_value_store::{
    CompareOutcome, MAX_VALUE_BYTES, PinValue, RouteValue, StoreError, StoreRead,
};

use super::{ParsePayload, ParseResult, StepOutput, ValueSummary};
use crate::router_probes::error::OperationError;

pub(super) fn parse_payload(payload: &ParsePayload) -> Result<ParseResult, OperationError> {
    let bytes = match payload {
        ParsePayload::Text { text } => text.as_bytes().to_vec(),
        ParsePayload::Pin { model_id_bytes } => {
            let length = usize::try_from(*model_id_bytes).map_err(|error| {
                OperationError::InvalidRequest {
                    message: error.to_string(),
                }
            })?;
            if length > MAX_VALUE_BYTES.saturating_add(1) {
                return Err(OperationError::InvalidRequest {
                    message: "the pin model id length is past the parser bound".to_owned(),
                });
            }
            let value = RouteValue::Pin(PinValue {
                model_id: "a".repeat(length),
            });
            serde_json::to_vec(&value).map_err(|source| StoreError::Encoding { source })?
        }
    };
    match parse_route_value(&bytes) {
        Ok(value) => Ok(ParseResult::Valid {
            value: summarize(&value),
        }),
        Err(StoreError::ValueTooLarge {
            bytes: reported,
            limit,
        }) => Ok(ParseResult::Invalid {
            bytes: reported,
            limit,
        }),
        Err(error) => Err(OperationError::from(error)),
    }
}

fn summarize(value: &RouteValue) -> ValueSummary {
    match value {
        RouteValue::Counter(counter) => ValueSummary::Counter { value: *counter },
        RouteValue::Spend(spend) => ValueSummary::Spend {
            amount: spend.to_string(),
        },
        RouteValue::Pin(pin) => ValueSummary::Pin {
            model_id_bytes: pin.model_id.len(),
        },
        RouteValue::SchedulerTuple(tuple) => ValueSummary::SchedulerTuple {
            priority: tuple.priority,
            request_id_bytes: tuple.request_id.len(),
        },
        RouteValue::Members(members) => ValueSummary::Members {
            members: members.iter().cloned().collect(),
        },
    }
}

pub(super) fn compare_output(outcome: &CompareOutcome) -> StepOutput {
    StepOutput::CompareAndSet {
        swapped: outcome.swapped,
        current: read_value(&outcome.current),
    }
}

pub(super) fn read_value(read: &StoreRead) -> Option<RouteValue> {
    match read {
        StoreRead::Missing => None,
        StoreRead::Value(value) => Some(value.clone()),
    }
}
