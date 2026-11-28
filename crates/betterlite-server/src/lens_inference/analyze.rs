use async_trait::async_trait;
use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_lens::models::messages::{ModelRequest, ModelResult};
use betterlite_logging::clickhouse::clickhouse_logging::context::LensAnalysis;
use betterlite_logging::logging_privacy::PrivacyContext;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::lens_billing::completion::{
    self, Billing, BillingError, CompletionAnswer, ReserveBudget,
};
use crate::lens_billing::internal_request::{Incoming, MessagePrivacyScope};
use crate::lens_billing::key_validation::{self, KeyResolver, KeyValidationError};
use crate::lens_inference::charge::{CostCalculator, CostError};
use crate::lens_inference::ports::{Clock, ModelCatalog};
use crate::lens_inference::quote::{self, Deployment, QuoteError};
use crate::lens_inference::request_body::{AnalysisBody, RequestIds};
use crate::lens_inference::reservation::{self, ReservationError, ReserveRequest};
use crate::lens_inference::settlement::{self, SettleRequest, SettlementError};

use self::answer::{answer_content, answer_cost};

mod answer;
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum AnalyzeError {
    #[error("No analysis models are configured")]
    NoModels,
    #[error("Assign an analysis key to this worker in Lens setup")]
    NoAnalysisKey,
    #[error(transparent)]
    Key(#[from] KeyValidationError),
    #[error("Analysis model is no longer available")]
    NoDeployments,
    #[error(transparent)]
    Quote(#[from] QuoteError),
    #[error(transparent)]
    Reservation(#[from] ReservationError),
    #[error(transparent)]
    Billing(#[from] BillingError),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    #[error(transparent)]
    Cost(#[from] CostError),
    #[error("the analysis model answered with no choices")]
    NoChoices,
}

impl AnalyzeError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::NoModels => 503,
            Self::NoAnalysisKey => 409,
            Self::Key(error) => error.status(),
            Self::NoDeployments => 400,
            Self::Quote(_) | Self::Settlement(_) | Self::Cost(_) | Self::NoChoices => 500,
            Self::Reservation(error) => error.status(),
            Self::Billing(error) => error.status(),
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Key(error) => error.detail(),
            Self::Reservation(error) => error.to_string(),
            Self::Billing(error) => error.detail(),
            Self::Quote(_)
            | Self::Settlement(_)
            | Self::Cost(_)
            | Self::NoModels
            | Self::NoAnalysisKey
            | Self::NoDeployments
            | Self::NoChoices => self.to_string(),
        }
    }
}

// every dependency analyze reaches, injected by B8682 so the Lens routes own no adapter
pub struct InferencePorts<'deps> {
    pub billing: &'deps Billing,
    pub keys: &'deps dyn KeyResolver,
    pub models: &'deps dyn ModelCatalog,
    pub prices: &'deps dyn ModelInfoSource,
    pub costs: &'deps dyn CostCalculator,
    pub clock: &'deps dyn Clock,
    pub lens_analysis: &'deps LensAnalysis,
}

#[derive(Clone, Copy)]
pub struct AnalysisInput<'call> {
    pub lens: &'call Lens,
    pub job: &'call Job,
    pub worker: &'call Worker,
    pub body: &'call ModelRequest,
    pub incoming: &'call Incoming,
}

struct Approved {
    key_id: String,
    team_id: Option<String>,
    deployments: Vec<Deployment>,
    estimate: Decimal,
}

struct BudgetReservation<'call, Db: Database> {
    repository: &'call LensRepository<Db>,
    lens_id: &'call str,
    job_id: &'call str,
    worker_id: &'call str,
    estimate: Decimal,
    month_now: DateTime<Utc>,
    clock: &'call dyn Clock,
}

#[async_trait]
impl<Db: Database> ReserveBudget for BudgetReservation<'_, Db> {
    async fn reserve(&self) -> Result<(), BillingError> {
        let request = ReserveRequest {
            lens_id: self.lens_id,
            job_id: self.job_id,
            worker_id: self.worker_id,
            month_now: self.month_now,
            lease_now: self.clock.now(),
            estimate: self.estimate,
        };
        reservation::reserve(self.repository, &request)
            .await
            .map_err(|error| BillingError::Http {
                status: error.status(),
                detail: error.to_string(),
            })
    }
}

// the router, key, model list and estimate every call needs before it may touch the budget
async fn approve(
    ports: &InferencePorts<'_>,
    input: &AnalysisInput<'_>,
) -> Result<Approved, AnalyzeError> {
    if !ports.models.configured() {
        return Err(AnalyzeError::NoModels);
    }
    let Some(key_id) = input.worker.analysis_key_id.as_deref() else {
        return Err(AnalyzeError::NoAnalysisKey);
    };
    let billing_key = key_validation::validate_key(ports.keys, Some(key_id)).await?;
    let team_id = billing_key.and_then(|key| key.team_id);
    let deployments = ports
        .models
        .deployments(&input.job.settings.model, team_id.as_deref());
    if deployments.is_empty() {
        return Err(AnalyzeError::NoDeployments);
    }
    let estimate = quote::quote(&deployments, &input.body.prompt, ports.prices)?;
    Ok(Approved {
        key_id: key_id.to_owned(),
        team_id,
        deployments,
        estimate,
    })
}

async fn run_scoped(
    ports: &InferencePorts<'_>,
    key_id: &str,
    input: &AnalysisInput<'_>,
    team_id: Option<&str>,
    reserve: &dyn ReserveBudget,
) -> Result<CompletionAnswer, AnalyzeError> {
    let data = AnalysisBody::new(
        &input.job.settings.model,
        &input.body.prompt,
        RequestIds {
            lens_id: &input.lens.id,
            run_id: &input.job.id,
            worker_id: &input.worker.id,
            team_id,
        },
    );
    let _lens = ports.lens_analysis.lens_analysis();
    let _privacy = MessagePrivacyScope::enter(
        &input.incoming.privacy,
        PrivacyContext {
            message_logging_disabled: true,
        },
    );
    completion::complete(ports.billing, key_id, &data, reserve, input.incoming)
        .await
        .map_err(AnalyzeError::from)
}

// inference.analyze: price the call, reserve the budget, run it under the lens scopes, then settle the real cost
pub async fn analyze<Db>(
    repository: &LensRepository<Db>,
    ports: &InferencePorts<'_>,
    input: AnalysisInput<'_>,
) -> Result<ModelResult, AnalyzeError>
where
    Db: Database,
{
    let approved = approve(ports, &input).await?;
    let month_now = ports.clock.now();
    let reserve = BudgetReservation {
        repository,
        lens_id: &input.lens.id,
        job_id: &input.job.id,
        worker_id: &input.worker.id,
        estimate: approved.estimate,
        month_now,
        clock: ports.clock,
    };
    let answer = run_scoped(
        ports,
        &approved.key_id,
        &input,
        approved.team_id.as_deref(),
        &reserve,
    )
    .await?;
    let content = answer_content(&answer)?;
    let cost = answer_cost(
        &answer,
        &approved.deployments,
        approved.estimate,
        ports.costs,
    )?;
    let settle = SettleRequest {
        lens_id: &input.lens.id,
        job_id: &input.job.id,
        month_now,
        estimate: approved.estimate,
        cost,
    };
    settlement::settle(repository, &settle).await?;
    Ok(ModelResult { content, cost })
}
