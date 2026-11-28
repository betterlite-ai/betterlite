use std::str::FromStr as _;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::lens_billing::internal_request::{
    self, ClientIp, Incoming, InternalRequest, RequestBuildError,
};
use crate::lens_inference::request_body::AnalysisBody;

mod tests;

const BUDGET_DETAIL: &str = "The analysis key or its owner has reached a budget limit";

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("The analysis key or its owner has reached a budget limit")]
    BudgetExceeded,
    #[error("{detail}")]
    Http { status: u16, detail: String },
    #[error("the in-process request could not be built: {0}")]
    Request(#[from] RequestBuildError),
    #[error("the billed cost header is not a decimal: {value}")]
    BilledCost { value: String },
}

impl BillingError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BudgetExceeded => 402,
            Self::Http { status, .. } => *status,
            Self::Request(_) | Self::BilledCost { .. } => 500,
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::BudgetExceeded => BUDGET_DETAIL.to_owned(),
            Self::Http { detail, .. } => detail.clone(),
            Self::Request(_) | Self::BilledCost { .. } => self.to_string(),
        }
    }
}

// authorize_internal_virtual_key, wired by B052 over the proxy's key authorization
#[async_trait]
pub trait InternalAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        key_id: &str,
        request: &InternalRequest,
        data: &AnalysisBody<'_>,
    ) -> Result<(), BillingError>;
}

// base_process_llm_request: the validated completion plus the x-litellm-response-cost header it answered with
pub struct ChatAnswer {
    pub response: ChatResponse,
    pub billed_cost: Option<String>,
}

#[async_trait]
pub trait ChatRunner: Send + Sync {
    async fn run(
        &self,
        request: &InternalRequest,
        data: &AnalysisBody<'_>,
    ) -> Result<ChatAnswer, BillingError>;
}

// reserve_budget: what analyze writes back before the chat call goes out
#[async_trait]
pub trait ReserveBudget: Send + Sync {
    async fn reserve(&self) -> Result<(), BillingError>;
}

// release_unbound_budget_reservation, wired by B8682 over release_unbound_reservation and the spend counters
#[async_trait]
pub trait ReservationRelease: Send + Sync {
    async fn release(&self, reservation: &ReservationHandle);
}

pub struct Billing {
    pub authorizer: Arc<dyn InternalAuthorizer>,
    pub runner: Arc<dyn ChatRunner>,
    pub releaser: Arc<dyn ReservationRelease>,
    pub client_ip: Arc<dyn ClientIp>,
}

#[derive(Debug)]
pub struct CompletionAnswer {
    pub response: ChatResponse,
    pub billed_cost: Option<Decimal>,
}

// x-litellm-response-cost: absent, empty and the string None all mean the pipeline billed nothing itself
fn parse_cost(billed: Option<&str>) -> Result<Option<Decimal>, BillingError> {
    match billed {
        None | Some("" | "None") => Ok(None),
        Some(text) => {
            let Ok(cost) = Decimal::from_str(text) else {
                return Err(BillingError::BilledCost {
                    value: text.to_owned(),
                });
            };
            Ok(Some(cost))
        }
    }
}

async fn release_unbound(billing: &Billing, request: &InternalRequest) {
    let Some(reservation) = lock(&request.reservation.0).clone() else {
        return;
    };
    if lock(&reservation.0).callback_bound {
        return;
    }
    billing.releaser.release(&reservation).await;
}

async fn dispatch(
    billing: &Billing,
    key_id: &str,
    data: &AnalysisBody<'_>,
    reserve: &dyn ReserveBudget,
    request: &InternalRequest,
) -> Result<CompletionAnswer, BillingError> {
    billing.authorizer.authorize(key_id, request, data).await?;
    reserve.reserve().await?;
    let answer = billing.runner.run(request, data).await?;
    let billed_cost = parse_cost(answer.billed_cost.as_deref())?;
    Ok(CompletionAnswer {
        response: answer.response,
        billed_cost,
    })
}

// billing.complete: authorize, reserve, then run the chat call, and release what was reserved whatever happens
pub async fn complete(
    billing: &Billing,
    key_id: &str,
    data: &AnalysisBody<'_>,
    reserve: &dyn ReserveBudget,
    incoming: &Incoming,
) -> Result<CompletionAnswer, BillingError> {
    let request = internal_request::build_request(data, incoming, billing.client_ip.as_ref())?;
    let outcome = dispatch(billing, key_id, data, reserve, &request).await;
    release_unbound(billing, &request).await;
    outcome
}
