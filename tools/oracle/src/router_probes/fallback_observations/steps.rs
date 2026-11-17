mod errors;
mod events;
mod headers;

use betterlite_router::fallback_observations::messages::{
    fallback_outcome_message, no_fallback_group_message,
};

use super::wire::{Input, Output};
use crate::router_probes::error::OperationError;

pub(super) async fn output(input: &Input) -> Result<Output, OperationError> {
    Ok(Output {
        details: errors::details(&input.details)?,
        no_fallback: input
            .no_fallback
            .iter()
            .map(|case| no_fallback_group_message(&case.lookup_groups, &case.fallbacks))
            .collect(),
        outcomes: input
            .outcomes
            .iter()
            .map(|case| {
                fallback_outcome_message(
                    case.model_group.as_deref(),
                    case.fallback_model_group.as_deref(),
                    &case.fallback_failure_detail,
                )
            })
            .collect(),
        headers: input.headers.iter().map(headers::header_found).collect(),
        events: events::events(&input.events).await,
    })
}
