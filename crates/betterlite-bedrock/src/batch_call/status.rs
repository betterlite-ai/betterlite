use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;
use serde_json::value::RawValue;

use crate::image_call::plan::CredentialSource;

use super::client::{control_plane, send};
use super::error::BatchCallError;
use super::logging::{CallEvent, CallLogger};
use super::model_job::project_job;
use super::region::resolve_region;
use super::reply::{client_error, decode_reply};
use super::request::{logging_url, sign_get};
use super::transport::{ControlPlaneFactory, ControlPlaneReply, accepted};

pub struct JobStatusCall<'source> {
    pub batch_id: &'source str,
    pub aws_region_name: Option<&'source str>,
    pub kwargs: &'source BTreeMap<String, Box<RawValue>>,
    pub now: SystemTime,
}

// GetModelInvocationJob: resolve the client exactly as cancel_batch does, log both sides, and
// project the job record onto the OpenAI batch shape
pub async fn retrieve_model_job_status<Source, Factory>(
    call: JobStatusCall<'_>,
    source: &Source,
    factory: &Factory,
    mut log: Option<&mut CallLogger<'_>>,
) -> Result<BatchObject, BatchCallError>
where
    Source: CredentialSource + Sync,
    Factory: ControlPlaneFactory,
{
    let region = resolve_region(call.aws_region_name, call.batch_id)?;
    let plane = control_plane(call.kwargs, &region, source, factory).await?;
    let api_base = logging_url(&region, call.batch_id);
    log_event(
        call.batch_id,
        CallEvent::Pre {
            api_base: &api_base,
        },
        &mut log,
    )?;
    let signed = sign_get(&plane.credentials, &region, call.batch_id, call.now)?;
    let reply = send(plane.transport, signed).await?;
    let response = job_response(&reply)?;
    log_event(
        call.batch_id,
        CallEvent::Post {
            response: &response,
        },
        &mut log,
    )?;
    project_job(&response, call.batch_id)
}

// the handler logs the batch id as its input and the job identifier as its complete input
fn log_event(
    batch_id: &str,
    event: CallEvent<'_>,
    log: &mut Option<&mut CallLogger<'_>>,
) -> Result<(), BatchCallError> {
    if let Some(context) = log.as_deref_mut() {
        context(batch_id, event)?;
    }
    Ok(())
}

// any reply of 300 and up is a botocore ClientError, and its body is parsed before the handler
fn job_response(reply: &ControlPlaneReply) -> Result<Json, BatchCallError> {
    if !accepted(reply.status) {
        return Err(client_error(reply));
    }
    decode_reply(reply)
}
