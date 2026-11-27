use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_config::collector::TracebackPolicy;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_db::database_failures::deadline::DbLookupDeadlineExceeded;
use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_logging::logging_metadata::error_info::ExceptionLogging;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::request_batch::lock as lock_reservation;
use thiserror::Error;

use super::enrich::{FailureIdentityLookup, enrich_failure_metadata};
use super::error::{FailureError, FailureErrorInformationError, failure_error_information};
use super::fields::{
    CallLog, FailureCostError, eligible_route, failure_response_cost, failure_start_time,
    lift_call_log,
};
use super::metadata::{
    ERROR_INFORMATION, FailureMetadataError, USED_CLIENT_OAUTH_TOKEN, merge_failure_metadata,
    persisted_metadata, trusted_failure_metadata, used_client_oauth_token,
};
use super::write::{FailureSpend, FailureSpendWrite};
use crate::spend::estimates::ports::Clock;
use crate::spend::settlement::ports::ReservationSettlement;

// general_settings.disable_error_logs, B1708's resolved traceback policy and B1713's spend sanitizing.
#[derive(Clone, Copy, Debug)]
pub struct FailureSettings {
    pub disable_error_logs: bool,
    pub tracebacks: TracebackPolicy,
    pub sanitize: SpendSanitizePolicy,
}

pub struct FailureReservation<'port> {
    pub settlement: &'port dyn ReservationSettlement,
    pub handle: &'port ReservationHandle,
}

pub struct FailurePorts<'port> {
    pub reservation: Option<FailureReservation<'port>>,
    pub writer: &'port dyn FailureSpendWrite,
    pub clock: &'port dyn Clock,
}

pub struct FailureRecord<'input> {
    pub request_data: &'input mut JsonObject,
    pub principal: &'input PrincipalFacts,
    pub logging: ExceptionLogging<'input>,
    pub traceback: Option<&'input str>,
    pub model_not_found: Option<&'input ProxyModelNotFoundError>,
    pub response_result: &'input str,
    pub deadline: Option<&'input DbLookupDeadlineExceeded>,
    pub call_log: Option<&'input CallLog>,
    pub identity: Option<&'input dyn FailureIdentityLookup>,
    pub resolve_missing_key_identity: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureRecording {
    Skipped,
    Recorded { charged: bool },
}

#[derive(Debug, Error)]
pub enum FailureRecordError {
    #[error(transparent)]
    Metadata(#[from] FailureMetadataError),
    #[error(transparent)]
    Information(#[from] FailureErrorInformationError),
    #[error(transparent)]
    Cost(#[from] FailureCostError),
}

// async_post_call_failure_hook: cleanup first, then attribution, then update_database with the exception as the response.
pub async fn record_failure(
    record: FailureRecord<'_>,
    settings: &FailureSettings,
    ports: &FailurePorts<'_>,
) -> Result<FailureRecording, FailureRecordError> {
    release_reservation(ports.reservation.as_ref()).await;
    if settings.disable_error_logs || !eligible_route(record.principal.request_route.as_deref()) {
        return Ok(FailureRecording::Skipped);
    }
    let trusted = trusted_metadata(&record, settings).await?;
    merge_failure_metadata(record.request_data, trusted);
    if let Some(call_log) = record.call_log {
        lift_call_log(record.request_data, call_log);
    }
    let end = SpendTimestamp::Aware(ports.clock.now().fixed_offset());
    let start = failure_start_time(record.request_data, record.call_log, end);
    let cost = failure_response_cost(record.request_data, persisted_metadata(record.request_data))?;
    let spend = FailureSpend {
        request_data: record.request_data,
        response_result: record.response_result,
        response_cost: cost,
        token: logged_api_key(record.principal),
        user_id: record.principal.user_id.as_deref(),
        end_user_id: record.principal.end_user_id.as_deref(),
        team_id: record.principal.team_id.as_deref(),
        org_id: record.principal.org_id.as_deref(),
        project_id: record.principal.project_id.as_deref(),
        start_time: start,
        end_time: end,
    };
    let charged = ports.writer.record(&spend).await;
    Ok(FailureRecording::Recorded { charged })
}

async fn trusted_metadata(
    record: &FailureRecord<'_>,
    settings: &FailureSettings,
) -> Result<JsonObject, FailureRecordError> {
    let mut trusted = trusted_failure_metadata(record.principal)?;
    let information = failure_error_information(&FailureError {
        logging: record.logging.clone(),
        model_not_found: record.model_not_found,
        traceback: record.traceback,
        policy: settings.sanitize,
        suppress_traceback: settings.tracebacks.suppress,
    })?;
    trusted.insert(ERROR_INFORMATION.to_owned(), information);
    if record.deadline.is_none() {
        enrich_failure_metadata(
            &mut trusted,
            record.identity,
            record.resolve_missing_key_identity,
        )
        .await;
    }
    let stamped = used_client_oauth_token(
        record.request_data,
        record.principal.request_route.as_deref(),
    )
    .map_or(Json::Null, Json::Bool);
    trusted.insert(USED_CLIENT_OAUTH_TOKEN.to_owned(), stamped);
    Ok(trusted)
}

async fn release_reservation(reservation: Option<&FailureReservation<'_>>) {
    let Some(active) = reservation else {
        return;
    };
    let Err(error) = active.settlement.release(active.handle).await else {
        return;
    };
    tracing::error!(%error, "Failed to release budget reservation during failure handling");
    match active.settlement.invalidate(active.handle).await {
        Ok(()) => lock_reservation(&active.handle.0).finalized = true,
        Err(invalidate_error) => tracing::error!(
            %invalidate_error,
            "Failed to invalidate budget reservation counters after failure release failed"
        ),
    }
}
