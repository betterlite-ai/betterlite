use std::fmt;

use tracing::{debug, error};

use crate::polling::host::{FetchInteraction, SettlementHost};
use crate::polling::registry::BackgroundPollRegistry;
use crate::polling::status::is_terminal_status;
use crate::types::response::InteractionsApiResponse;

mod tests;

pub async fn release_open_reservation<Host>(host: &Host)
where
    Host: SettlementHost,
{
    let Some(reservation) = host.budget_reservation() else {
        return;
    };
    // a failed release must not fail the caller: the spend counters expire on their own
    if let Err(source) = host.release_reservation(&reservation).await {
        error!(
            error = %source,
            "Failed to release budget reservation for an unbilled background interaction"
        );
    }
}

pub async fn bill_settled_interaction<Host>(
    host: &Host,
    response: &InteractionsApiResponse,
) -> Result<(), Host::BillingError>
where
    Host: SettlementHost,
{
    if let Err(source) = host.log_background_completion(response).await {
        // the claim is already spent, so nothing retries billing and the reservation must not stay pinned
        release_open_reservation(host).await;
        return Err(source);
    }
    Ok(())
}

pub async fn settle_before_delete<Host, Error>(
    registry: &BackgroundPollRegistry<Host>,
    interaction_id: &str,
    fetch: FetchInteraction<Host, Error>,
) -> Result<(), Host::BillingError>
where
    Host: SettlementHost + Clone,
    Error: fmt::Display,
{
    let Some(context) = registry.active_poll(interaction_id) else {
        return Ok(());
    };
    let response = match fetch(context.clone()).await {
        Ok(response) => response,
        Err(source) => {
            debug!(
                interaction_id = %interaction_id,
                error = %source,
                "Could not fetch background interaction before delete, releasing its reservation"
            );
            if context.settlement.claim_settlement() {
                release_open_reservation(&context.host).await;
            }
            return Ok(());
        }
    };
    // the poll task may have settled the interaction while the fetch was in flight
    if !context.settlement.claim_settlement() {
        return Ok(());
    }
    if is_terminal_status(response.status.as_deref()) && response.usage.is_some() {
        return bill_settled_interaction(&context.host, &response).await;
    }
    release_open_reservation(&context.host).await;
    Ok(())
}
