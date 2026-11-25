use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use rust_decimal::Decimal;

use super::accounting::{SpendAccounting, SpendAccountingClosed};
use super::{DbSpendUpdateWriter, SpendWriterFailure, SpendWriterOperation};
use crate::key_user_writes::hash::hash_token_if_needed;
use crate::redis_spend::codec::{SpendBufferError, decode_log_record};
use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::request_usage::TurnEnvironment;
use crate::request_usage::discovery::enqueue_tool_registry_upsert;
use crate::request_usage::enqueue::enqueue_tool_usage;
use crate::request_usage::turn_enqueue::enqueue_autorouter_turn;
use crate::spend_log_storage::batch_claim::{
    BatchClaimTable, SpendLogAttachment, record_spend_log,
};
use crate::spend_queues::entity::SpendAmount;

mod tests;

pub type PayloadFuture<'call, Failure> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, Failure>> + Send + 'call>>;

// get_logging_payload of B1716 behind a port: the server builds the spend log and the db writer only reads its bytes.
pub trait SpendPayloadSource {
    type Error: StdError + Send + Sync + 'static;

    fn build(&self) -> PayloadFuture<'_, Self::Error>;
}

// update_database's arguments besides the payload: what the caller knows about the request and the routing catalog the turn enqueue prices with.
pub struct SpendUpdateRequest<'call, C> {
    pub response_cost: Option<SpendAmount>,
    pub token: Option<&'call str>,
    pub user_id: Option<&'call str>,
    pub end_user_id: Option<&'call str>,
    pub team_id: Option<&'call str>,
    pub org_id: Option<&'call str>,
    pub project_id: Option<&'call str>,
    pub kwargs: Option<&'call JsonObject>,
    pub response_names: &'call [String],
    pub realtime_names: &'call [String],
    pub model_access_groups: &'call [String],
    pub turn_environment: Option<&'call TurnEnvironment<'call, C>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SpendLogBuildError {
    #[error("the spend log payload is not a JSON object")]
    NotAnObject,
    #[error("the spend log payload could not be read")]
    Decode(#[source] SpendBufferError),
    #[error("the spend log record could not be written as JSON")]
    Encode(#[source] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
enum RequestError<PayloadError>
where
    PayloadError: StdError + Send + Sync + 'static,
{
    #[error("the spend log payload could not be built")]
    Payload {
        #[source]
        source: PayloadError,
    },
    #[error(transparent)]
    Build(#[from] SpendLogBuildError),
    #[error("the spend accounting work was not admitted")]
    Admission(#[from] SpendAccountingClosed),
}

impl<Table> DbSpendUpdateWriter<'_, Table>
where
    Table: BatchClaimTable + ?Sized,
{
    // update_database answers whether the caller still owns this request's charge: False only when another retrieve claimed its batch cost row.
    #[must_use]
    pub async fn update_database<C, P>(
        &self,
        payloads: &P,
        request: &SpendUpdateRequest<'_, C>,
    ) -> bool
    where
        C: SavingsPricingContext,
        P: SpendPayloadSource + Sync,
    {
        if self.settings.disable_spend_updates {
            return true;
        }
        match self.write_request(payloads, request).await {
            Ok(claimed) => claimed,
            Err(error) => {
                self.report(SpendWriterOperation::UpdateDatabase, request, &error);
                true
            }
        }
    }

    fn report<C>(
        &self,
        operation: SpendWriterOperation,
        request: &SpendUpdateRequest<'_, C>,
        error: &(dyn StdError + Send + Sync),
    ) {
        self.failures.report(SpendWriterFailure {
            operation,
            response_cost: request.response_cost,
            token: request.token,
            user_id: request.user_id,
            team_id: request.team_id,
            org_id: request.org_id,
            end_user_id: request.end_user_id,
            error,
        });
    }

    async fn write_request<C, P>(
        &self,
        payloads: &P,
        request: &SpendUpdateRequest<'_, C>,
    ) -> Result<bool, RequestError<P::Error>>
    where
        C: SavingsPricingContext,
        P: SpendPayloadSource + Sync,
    {
        let built = payloads
            .build()
            .await
            .map_err(|source| RequestError::Payload { source })?;
        let row = spend_log_row(
            &built,
            request.response_cost,
            request.org_id,
            request.team_id,
        )?;
        let hashed_token = request.token.map(hash_token_if_needed);
        let queued = self.attachment.as_ref().map(|source| SpendLogAttachment {
            table: source.table,
            queue: source.queue,
        });
        let disable_logs = self.settings.disable_spend_logs != Some(false);
        if !record_spend_log(queued, &row, disable_logs).await {
            return Ok(false);
        }
        let serialized = serde_json::to_vec(&row).map_err(SpendLogBuildError::Encode)?;
        self.enqueue_log_dependent_usage(&serialized, request).await;
        let work = SpendAccounting {
            response_cost: request.response_cost,
            user_id: request.user_id.map(ToOwned::to_owned),
            hashed_token: hashed_token.clone(),
            team_id: request.team_id.map(ToOwned::to_owned),
            org_id: request.org_id.map(ToOwned::to_owned),
            project_id: request.project_id.map(ToOwned::to_owned),
            end_user_id: request.end_user_id.map(ToOwned::to_owned),
            model_access_groups: request.model_access_groups.to_vec(),
            payload: row,
        };
        self.admission.admit(work).await?;
        self.discover_tools(request, hashed_token.as_deref())
            .await?;
        Ok(true)
    }

    // The source's own else branch: only False writes the log-dependent usage, while entity and daily accounting still run.
    async fn enqueue_log_dependent_usage<C>(
        &self,
        payload: &[u8],
        request: &SpendUpdateRequest<'_, C>,
    ) where
        C: SavingsPricingContext,
    {
        if self.settings.disable_spend_logs != Some(false) {
            tracing::debug!(
                "disable_spend_logs is not false; skipping the spend log usage and auto-router turn enqueues"
            );
            return;
        }
        if let Some(queue) = self.queues.tool_usage {
            enqueue_tool_usage(
                queue,
                payload,
                request.response_names,
                request.realtime_names,
            )
            .await;
        }
        if let (Some(queues), Some(environment)) = (self.queues.turns, request.turn_environment) {
            enqueue_autorouter_turn(Some(queues), environment, payload).await;
        }
    }

    // _enqueue_tool_registry_upsert over the kwargs the request arrived with, after the charge was admitted.
    async fn discover_tools<C>(
        &self,
        request: &SpendUpdateRequest<'_, C>,
        hashed_token: Option<&str>,
    ) -> Result<(), SpendLogBuildError>
    where
        C: SavingsPricingContext,
    {
        let Some(queue) = self.queues.discovery else {
            return Ok(());
        };
        let serialized = request
            .kwargs
            .map(serde_json::to_vec)
            .transpose()
            .map_err(SpendLogBuildError::Encode)?;
        enqueue_tool_registry_upsert(
            queue,
            serialized.as_deref(),
            hashed_token,
            request.team_id,
            request.response_names,
        )
        .await;
        Ok(())
    }
}

// The payload the source builds, then `payload["spend"] = response_cost or 0.0` and the nonempty organization/team overrides.
fn spend_log_row(
    bytes: &[u8],
    response_cost: Option<SpendAmount>,
    org_id: Option<&str>,
    team_id: Option<&str>,
) -> Result<SpendLogRow, SpendLogBuildError> {
    let mut row = decode_log_record(bytes)
        .map_err(SpendLogBuildError::Decode)?
        .ok_or(SpendLogBuildError::NotAnObject)?;
    let spend = response_cost.map_or(Decimal::ZERO, |amount| amount.0);
    row.insert("spend".to_owned(), SpendLogValue::Decimal(spend));
    if let Some(org) = org_id.filter(|id| !id.is_empty()) {
        row.insert(
            "organization_id".to_owned(),
            SpendLogValue::Text(org.to_owned()),
        );
    }
    if let Some(team) = team_id.filter(|id| !id.is_empty()) {
        row.insert("team_id".to_owned(), SpendLogValue::Text(team.to_owned()));
    }
    Ok(row)
}
