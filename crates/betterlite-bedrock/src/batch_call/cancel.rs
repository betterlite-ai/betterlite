use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_openai::batches::records::{BatchObject, BatchStatus};
use serde_json::value::RawValue;

use crate::image_call::plan::CredentialSource;

use super::client::{control_plane, send};
use super::error::BatchCallError;
use super::logging::CallLogger;
use super::region::resolve_region;
use super::reply::{client_error, decode_reply};
use super::request::sign_stop;
use super::status::{JobStatusCall, retrieve_model_job_status};
use super::transport::{ControlPlaneFactory, ControlPlaneReply};

const IDEMPOTENT_STATUSES: [BatchStatus; 5] = [
    BatchStatus::Cancelling,
    BatchStatus::Cancelled,
    BatchStatus::Completed,
    BatchStatus::Failed,
    BatchStatus::Expired,
];

const TOLERATED_ERROR_CODES: [&str; 2] = ["ValidationException", "ConflictException"];

pub struct CancelBatchCall<'source> {
    pub batch_id: &'source str,
    pub aws_region_name: Option<&'source str>,
    pub kwargs: &'source BTreeMap<String, Box<RawValue>>,
    pub now: SystemTime,
}

// StopModelInvocationJob: a stop error is tolerated only when the job's fresh status is terminal
pub async fn cancel_batch<Source, Factory>(
    call: CancelBatchCall<'_>,
    source: &Source,
    factory: &Factory,
    log: Option<&mut CallLogger<'_>>,
) -> Result<BatchObject, BatchCallError>
where
    Source: CredentialSource + Sync,
    Factory: ControlPlaneFactory,
{
    let region = resolve_region(call.aws_region_name, call.batch_id)?;
    let plane = control_plane(call.kwargs, &region, source, factory).await?;
    let signed = sign_stop(&plane.credentials, &region, call.batch_id, call.now)?;
    let reply = send(plane.transport, signed).await?;
    let Some(failure) = stop_failure(&reply)? else {
        return current_status(&call, &region, source, factory, log).await;
    };
    if !tolerated(&failure) {
        return Err(failure);
    }
    let current = current_status(&call, &region, source, factory, log).await?;
    if IDEMPOTENT_STATUSES.contains(&current.status) {
        return Ok(current);
    }
    Err(failure)
}

// a stop reply is parsed before it is accepted, and any reply of 300 and up is a ClientError
fn stop_failure(reply: &ControlPlaneReply) -> Result<Option<BatchCallError>, BatchCallError> {
    if reply.status >= 300 {
        return Ok(Some(client_error(reply)));
    }
    decode_reply(reply)?;
    Ok(None)
}

fn tolerated(failure: &BatchCallError) -> bool {
    matches!(
        failure,
        BatchCallError::Client { code: Some(code), .. }
            if TOLERATED_ERROR_CODES.contains(&code.as_str())
    )
}

async fn current_status<Source, Factory>(
    call: &CancelBatchCall<'_>,
    region: &str,
    source: &Source,
    factory: &Factory,
    log: Option<&mut CallLogger<'_>>,
) -> Result<BatchObject, BatchCallError>
where
    Source: CredentialSource + Sync,
    Factory: ControlPlaneFactory,
{
    retrieve_model_job_status(
        JobStatusCall {
            batch_id: call.batch_id,
            aws_region_name: Some(region),
            kwargs: call.kwargs,
            now: call.now,
        },
        source,
        factory,
        log,
    )
    .await
}
