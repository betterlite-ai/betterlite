use std::convert::Infallible;
use std::error::Error;

use betterlite_cost::adjustments::AdjustmentError;
use betterlite_models::billing_text::error::BillingTextError;
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::protocol_cost::error::ProtocolCostError;
use thiserror::Error;

use crate::cost::call_price::policy::error::PolicyError;
use crate::cost::context::error::BillingContextError;
use crate::cost::image_dispatch::error::ImageDispatchError;
use crate::cost::realtime::error::RealtimeCostError;

#[derive(Debug, Error)]
pub enum ReplicateError {
    #[error("the Replicate charge is too large to represent")]
    Overflow,
    #[error("the clock reads a time before the Unix epoch")]
    ClockBeforeEpoch,
}

#[derive(Debug, Error)]
pub enum SpecialChargeError<ImageError = Infallible>
where
    ImageError: Error + 'static,
{
    #[error("model is None and does not exist in the passed completion response")]
    ModelMissing,
    #[error(
        "usage object and custom_llm_provider must be provided for realtime stream cost calculation"
    )]
    RealtimeInputs,
    #[error("the search charge is too large to represent")]
    Overflow,
    #[error(transparent)]
    Context(#[from] BillingContextError),
    #[error(transparent)]
    Image(#[from] ImageDispatchError<ImageError>),
    #[error(transparent)]
    Video(#[from] DurationCostError),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Adjustment(#[from] AdjustmentError),
    #[error(transparent)]
    Realtime(#[from] RealtimeCostError),
    #[error(transparent)]
    Protocol(#[from] ProtocolCostError),
    #[error(transparent)]
    Text(#[from] BillingTextError),
    #[error(transparent)]
    Replicate(#[from] ReplicateError),
}
