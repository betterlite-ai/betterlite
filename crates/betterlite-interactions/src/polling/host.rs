use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::types::response::InteractionsApiResponse;

use super::context::InteractionPollContext;

pub type SettlementFuture<'host, Value> = Pin<Box<dyn Future<Output = Value> + Send + 'host>>;

pub type FetchFuture<Error> =
    Pin<Box<dyn Future<Output = Result<InteractionsApiResponse, Error>> + Send>>;

pub type FetchInteraction<Host, Error> =
    Arc<dyn Fn(InteractionPollContext<Host>) -> FetchFuture<Error> + Send + Sync>;

// the create's logging object implements this; the reservation type stays the logging side's
pub trait SettlementHost: Send + Sync {
    type Reservation: Send;
    type BillingError: Error + Send + Sync + 'static;
    type ReleaseError: Error + Send + Sync + 'static;

    fn budget_reservation(&self) -> Option<Self::Reservation>;

    fn log_background_completion<'host>(
        &'host self,
        response: &'host InteractionsApiResponse,
    ) -> SettlementFuture<'host, Result<(), Self::BillingError>>;

    fn release_reservation<'host>(
        &'host self,
        reservation: &'host Self::Reservation,
    ) -> SettlementFuture<'host, Result<(), Self::ReleaseError>>;
}
