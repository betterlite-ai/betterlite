use async_trait::async_trait;
use betterlite_config::collector::TracebackPolicy;
use betterlite_db::spend_log_storage::batch_claim::BatchClaimTable;
use betterlite_db::spend_queues::entity::SpendAmount;
use betterlite_db::spend_writer::DbSpendUpdateWriter;
use betterlite_db::spend_writer::request::{PayloadFuture, SpendPayloadSource, SpendUpdateRequest};
use betterlite_logging::payload::deps::PayloadClock;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_models::model_registration::JsonObject;
use chrono::Utc;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::watch;

use super::fields::{completion_start, failure_response_result};
use crate::savings::Context as SpendContext;
use crate::spend::payload::{
    PayloadRequest, SpendCache, SpendPayloadError, SpendPayloadSettings, SpendResponse,
    get_logging_payload,
};

pub struct FailureSpend<'spend> {
    pub request_data: &'spend JsonObject,
    pub response_result: &'spend str,
    pub response_cost: Decimal,
    pub token: Option<&'spend str>,
    pub user_id: Option<&'spend str>,
    pub end_user_id: Option<&'spend str>,
    pub team_id: Option<&'spend str>,
    pub org_id: Option<&'spend str>,
    pub project_id: Option<&'spend str>,
    pub start_time: SpendTimestamp,
    pub end_time: SpendTimestamp,
}

// db_spend_update_writer.update_database: true while this row still owns its charge.
#[async_trait]
pub trait FailureSpendWrite: Send + Sync {
    async fn record(&self, spend: &FailureSpend<'_>) -> bool;
}

pub struct DatabaseFailureSpendWrite<'call, 'work, Table: BatchClaimTable + ?Sized> {
    pub writer: &'call DbSpendUpdateWriter<'work, Table>,
    pub settings: SpendPayloadSettings,
    pub cache: Option<&'call (dyn SpendCache + Sync)>,
    pub clock: &'call dyn PayloadClock,
    pub session_uuid: &'call str,
    pub tracebacks: &'call watch::Receiver<TracebackPolicy>,
}

#[async_trait]
impl<Table> FailureSpendWrite for DatabaseFailureSpendWrite<'_, '_, Table>
where
    Table: BatchClaimTable + ?Sized,
{
    async fn record(&self, spend: &FailureSpend<'_>) -> bool {
        let payloads = FailurePayloads {
            kwargs: spend.request_data,
            response_result: spend.response_result,
            start_time: spend.start_time,
            end_time: spend.end_time,
            cache: self.cache,
            clock: self.clock,
            session_uuid: self.session_uuid,
            tracebacks: self.tracebacks,
            settings: self.settings,
        };
        let update = SpendUpdateRequest::<SpendContext<'static>> {
            response_cost: Some(SpendAmount(spend.response_cost)),
            token: spend.token,
            user_id: spend.user_id,
            end_user_id: spend.end_user_id,
            team_id: spend.team_id,
            org_id: spend.org_id,
            project_id: spend.project_id,
            kwargs: Some(spend.request_data),
            response_names: &[],
            realtime_names: &[],
            model_access_groups: &[],
            turn_environment: None,
        };
        self.writer.update_database(&payloads, &update).await
    }
}

#[derive(Debug, Error)]
pub enum FailureBuildError {
    #[error(transparent)]
    Payload(#[from] SpendPayloadError),
    #[error("the failure spend log could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

struct FailurePayloads<'spend> {
    kwargs: &'spend JsonObject,
    response_result: &'spend str,
    start_time: SpendTimestamp,
    end_time: SpendTimestamp,
    cache: Option<&'spend (dyn SpendCache + Sync)>,
    clock: &'spend dyn PayloadClock,
    session_uuid: &'spend str,
    tracebacks: &'spend watch::Receiver<TracebackPolicy>,
    settings: SpendPayloadSettings,
}

impl SpendPayloadSource for FailurePayloads<'_> {
    type Error = FailureBuildError;

    fn build(&self) -> PayloadFuture<'_, Self::Error> {
        Box::pin(async move {
            let response = failure_response_result(self.response_result);
            let cache: Option<&dyn SpendCache> = match self.cache {
                Some(shared) => Some(shared),
                None => None,
            };
            let request = PayloadRequest {
                kwargs: self.kwargs,
                response: SpendResponse::Mapping(&response),
                start_time: self.start_time,
                end_time: self.end_time,
                completion_start_time: completion_start(self.kwargs),
                router: None,
                cache,
                clock: self.clock,
                session_uuid: self.session_uuid,
                tracebacks: self.tracebacks,
            };
            let payload = get_logging_payload(&request, &self.settings, &Utc)?;
            serde_json::to_vec(&payload).map_err(|source| FailureBuildError::Encode { source })
        })
    }
}
