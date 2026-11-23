use thiserror::Error;

use crate::auth_request_models::catalog::CatalogError;
use crate::reservation_admission::error::AdmissionError;
use crate::reservation_cost::error::ReservationCostError;
use crate::reservation_counters::error::ReservationCounterError;

// reserve_budget_for_request's failures, each the collaborator's own error carried unchanged
#[derive(Debug, Error)]
pub enum ReservationRequestError {
    #[error(transparent)]
    Counters(#[from] ReservationCounterError),
    #[error(transparent)]
    Model(#[from] CatalogError),
    #[error(transparent)]
    Cost(#[from] ReservationCostError),
    #[error(transparent)]
    Admission(#[from] AdmissionError),
}
