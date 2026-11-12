use std::fmt;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::sleep;
use tracing::{debug, error, warn};

use crate::types::response::InteractionsApiResponse;

use super::context::InteractionPollContext;
use super::host::{FetchInteraction, SettlementHost};
use super::intervals::PollIntervals;
use super::status::{is_pollable_status, is_terminal_status};

pub(super) async fn poll_background_cost<Host, Error>(
    context: InteractionPollContext<Host>,
    fetch: FetchInteraction<Host, Error>,
) where
    Host: SettlementHost + Clone,
    Error: fmt::Display,
{
    let mut last_seen_status = None;
    if poll_until_terminal(&context, &fetch, &mut last_seen_status).await {
        return;
    }
    if !context.settlement.claim_settlement() {
        return;
    }
    log_gave_up(&context, last_seen_status.as_deref());
    release_open_reservation(&context.host).await;
}

async fn poll_until_terminal<Host, Error>(
    context: &InteractionPollContext<Host>,
    fetch: &FetchInteraction<Host, Error>,
    last_seen_status: &mut Option<String>,
) -> bool
where
    Host: SettlementHost + Clone,
    Error: fmt::Display,
{
    let intervals = PollIntervals {
        interval: context.initial_interval,
        maximum: context.max_interval,
        elapsed: Duration::ZERO,
        timeout: context.timeout,
    };
    for interval in intervals {
        sleep(interval).await;
        if context.settlement.settled.load(Ordering::Acquire) {
            return true;
        }
        let response = match fetch(context.clone()).await {
            Ok(response) => response,
            Err(source) => {
                debug!(
                    interaction_id = %context.interaction_id,
                    error = %source,
                    "background interaction cost poll failed, will retry"
                );
                continue;
            }
        };
        last_seen_status.clone_from(&response.status);
        if !is_terminal_status(response.status.as_deref()) {
            continue;
        }
        if !context.settlement.claim_settlement() {
            return true;
        }
        settle_terminal(context, &response).await;
        return true;
    }
    false
}

async fn settle_terminal<Host>(
    context: &InteractionPollContext<Host>,
    response: &InteractionsApiResponse,
) where
    Host: SettlementHost,
{
    if response.usage.is_none() {
        release_open_reservation(&context.host).await;
        return;
    }
    if let Err(source) = context.host.log_background_completion(response).await {
        release_open_reservation(&context.host).await;
        error!(
            interaction_id = %context.interaction_id,
            error = %source,
            "failed to bill a settled background interaction"
        );
    }
}

async fn release_open_reservation<Host>(host: &Host)
where
    Host: SettlementHost,
{
    let Some(reservation) = host.budget_reservation() else {
        return;
    };
    if let Err(source) = host.release_reservation(&reservation).await {
        error!(
            error = %source,
            "Failed to release budget reservation for an unbilled background interaction"
        );
    }
}

fn log_gave_up<Host>(context: &InteractionPollContext<Host>, last_seen_status: Option<&str>) {
    match last_seen_status {
        Some(status) if !is_pollable_status(Some(status)) => error!(
            interaction_id = %context.interaction_id,
            timeout = ?context.timeout,
            status = %status,
            "gave up cost polling for a background interaction: its last status is in neither the pollable nor the terminal set, so its usage will not be tracked"
        ),
        _ => warn!(
            interaction_id = %context.interaction_id,
            timeout = ?context.timeout,
            "gave up cost polling for a background interaction; its usage will not be tracked"
        ),
    }
}
