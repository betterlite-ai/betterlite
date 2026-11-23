use std::time::Duration;

use betterlite_azure::fine_tuning::ensure_training_type;
use betterlite_core::api::fine_tuning_cancel_call::{
    AzureFineTuningCancel, FineTuningCancelTarget, OpenAiFineTuningCancel,
};
use betterlite_core::api::fine_tuning_create::{
    AzureCreateCall, FineTuningCreateTarget, OpenAiCreateCall,
};
use betterlite_core::api::fine_tuning_read::{
    AzureReadCall, OpenAiReadCall, ReadOperation, ReadTarget,
};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::error::WireError;
use super::azure::{azure_headers, azure_jobs_url, azure_query, azure_version};
use super::insert_header;

const JSON_CONTENT: &str = "application/json";
const DEFAULT_DEADLINE: Duration = Duration::from_secs(600);

// One fine-tuning call the native side can send, whichever provider or operation named it.
pub(crate) enum WireCall<'call> {
    OpenAiCreate(&'call OpenAiCreateCall),
    AzureCreate(&'call AzureCreateCall),
    OpenAiRead(&'call OpenAiReadCall<'call>),
    AzureRead(&'call AzureReadCall<'call>),
    OpenAiCancel(&'call OpenAiFineTuningCancel<'call>),
    AzureCancel(&'call AzureFineTuningCancel<'call>),
}

// The create target's two supported providers; the vertex gap stays the caller's to answer.
pub(crate) fn create_call(target: &FineTuningCreateTarget) -> Result<WireCall<'_>, WireError> {
    match target {
        FineTuningCreateTarget::OpenAi(call) => Ok(WireCall::OpenAiCreate(call)),
        FineTuningCreateTarget::Azure(call) => Ok(WireCall::AzureCreate(call)),
        FineTuningCreateTarget::Vertex(_) => Err(WireError::Vertex),
    }
}

pub(crate) fn read_call<'call>(target: &'call ReadTarget<'call>) -> WireCall<'call> {
    match target {
        ReadTarget::OpenAi(call) => WireCall::OpenAiRead(call),
        ReadTarget::Azure(call) => WireCall::AzureRead(call),
    }
}

pub(crate) fn cancel_call<'call>(target: &'call FineTuningCancelTarget<'call>) -> WireCall<'call> {
    match target {
        FineTuningCancelTarget::OpenAi(call) => WireCall::OpenAiCancel(call),
        FineTuningCancelTarget::Azure(call) => WireCall::AzureCancel(call),
    }
}

// The one request builder: every operation and provider fills the same outbound shape.
pub(crate) fn request_for(
    call: WireCall<'_>,
    integrations: Option<&[Json]>,
) -> Result<(OutboundRequest, bool), WireError> {
    match call {
        WireCall::OpenAiCreate(call) => Ok((
            OutboundRequest {
                method: Method::POST,
                url: openai_jobs_url(&call.api_base, "")?,
                query: Vec::new(),
                headers: openai_headers(call.api_key.as_ref(), call.organization.as_deref(), true)?,
                body: RequestBody::Json(create_body(&call.data, false, integrations)?),
                deadline: deadline(&call.timeout),
            },
            false,
        )),
        WireCall::AzureCreate(call) => {
            let (key, api_base, version) = azure_api(
                call.api_key.as_ref(),
                call.api_base.as_deref(),
                call.api_version.as_deref(),
            )?;
            let mut headers = azure_headers(key, version)?;
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
            Ok((
                OutboundRequest {
                    method: Method::POST,
                    url: azure_jobs_url(api_base, version, "")?,
                    query: azure_query(version),
                    headers,
                    body: RequestBody::Json(create_body(&call.data, true, integrations)?),
                    deadline: deadline(&call.timeout),
                },
                true,
            ))
        }
        WireCall::OpenAiRead(call) => Ok((
            OutboundRequest {
                method: Method::GET,
                url: openai_jobs_url(&call.api_base, &read_suffix(call.values.operation))?,
                query: paging(call.values.operation),
                headers: openai_headers(call.api_key, call.organization, false)?,
                body: RequestBody::Empty,
                deadline: deadline(call.values.timeout),
            },
            false,
        )),
        WireCall::AzureRead(call) => {
            let (key, api_base, version) =
                azure_api(call.api_key, call.api_base, call.api_version)?;
            let mut query = azure_query(version);
            query.extend(paging(call.values.operation));
            Ok((
                OutboundRequest {
                    method: Method::GET,
                    url: azure_jobs_url(api_base, version, &read_suffix(call.values.operation))?,
                    query,
                    headers: azure_headers(key, version)?,
                    body: RequestBody::Empty,
                    deadline: deadline(call.values.timeout),
                },
                true,
            ))
        }
        WireCall::OpenAiCancel(call) => Ok((
            OutboundRequest {
                method: Method::POST,
                url: openai_jobs_url(&call.api_base, &format!("/{}/cancel", call.values.job_id))?,
                query: Vec::new(),
                headers: openai_headers(call.api_key, call.organization, false)?,
                body: RequestBody::Empty,
                deadline: deadline(call.values.timeout),
            },
            false,
        )),
        WireCall::AzureCancel(call) => {
            let (key, api_base, version) =
                azure_api(call.api_key, call.api_base, call.api_version)?;
            let suffix = format!("/{}/cancel", call.values.job_id);
            Ok((
                OutboundRequest {
                    method: Method::POST,
                    url: azure_jobs_url(api_base, version, &suffix)?,
                    query: azure_query(version),
                    headers: azure_headers(key, version)?,
                    body: RequestBody::Empty,
                    deadline: deadline(call.values.timeout),
                },
                true,
            ))
        }
    }
}

// The openai client's own headers: the bearer token, the organization, and JSON on a body-carrying call.
fn openai_headers(
    key: Option<&SecretString>,
    organization: Option<&str>,
    is_json: bool,
) -> Result<HeaderMap, WireError> {
    let Some(key) = key else {
        return Err(WireError::MissingOpenAiKey);
    };
    let mut headers = HeaderMap::new();
    if is_json {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    }
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {}", key.expose_secret()),
    )?;
    if let Some(organization) = organization {
        insert_header(&mut headers, "openai-organization", organization)?;
    }
    Ok(headers)
}

// The two parts every Azure call needs, and the version request_for then reads.
fn azure_api<'call>(
    key: Option<&'call SecretString>,
    api_base: Option<&'call str>,
    api_version: Option<&'call str>,
) -> Result<(&'call SecretString, &'call str, &'call str), WireError> {
    let Some(key) = key else {
        return Err(WireError::MissingAzureKey);
    };
    let Some(api_base) = api_base else {
        return Err(WireError::MissingAzureBase);
    };
    Ok((key, api_base, azure_version(api_version)))
}

fn openai_jobs_url(api_base: &str, suffix: &str) -> Result<Url, WireError> {
    Url::parse(&format!(
        "{}/fine_tuning/jobs{suffix}",
        api_base.trim_end_matches('/')
    ))
    .map_err(|source| WireError::Url { source })
}

// The SDK merges create.extra_body into the request body, and Azure always sends a trainingType.
pub(super) fn create_body(
    data: &FineTuneCreate,
    is_azure: bool,
    integrations: Option<&[Json]>,
) -> Result<Vec<u8>, WireError> {
    let mut record = data.clone();
    if is_azure {
        ensure_training_type(&mut record);
    }
    let bytes = serde_json::to_vec(&record).map_err(|source| WireError::Encode { source })?;
    let mut body =
        serde_json::from_slice::<Json>(&bytes).map_err(|source| WireError::Body { source })?;
    if let Json::Object(fields) = &mut body {
        // the loose list[str] the Python call declares never reaches the stricter SDK record
        if let Some(values) = integrations {
            fields.insert("integrations".to_owned(), Json::List(values.to_vec()));
        }
        if let Some(Json::Object(extra)) = fields.shift_remove("extra_body") {
            fields.extend(extra);
        }
    }
    serde_json::to_vec(&body).map_err(|source| WireError::Encode { source })
}

fn read_suffix(operation: ReadOperation<'_>) -> String {
    match operation {
        ReadOperation::List { .. } => String::new(),
        ReadOperation::Retrieve { job_id } => format!("/{job_id}"),
    }
}

// the SDK drops a null, empty or missing pagination value and sends a zero limit as "0"
fn paging(operation: ReadOperation<'_>) -> Vec<(String, String)> {
    let ReadOperation::List { after, limit } = operation else {
        return Vec::new();
    };
    let mut query = Vec::new();
    if let Some(cursor) = after.filter(|value| !value.is_empty()) {
        query.push(("after".to_owned(), cursor.to_owned()));
    }
    if let Some(page_size) = limit {
        query.push(("limit".to_owned(), page_size.to_string()));
    }
    query
}

pub(super) fn deadline(timeout: &FineTuningTimeout) -> Duration {
    let seconds = match timeout {
        FineTuningTimeout::Seconds(value) => Some(*value),
        FineTuningTimeout::Structured(found) => {
            found.read.or(found.connect).or(found.write).or(found.pool)
        }
    };
    seconds
        .and_then(|value| Duration::try_from_secs_f64(value).ok())
        .filter(|duration| !duration.is_zero())
        .unwrap_or(DEFAULT_DEADLINE)
}
