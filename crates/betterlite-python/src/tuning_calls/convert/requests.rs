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
use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::error::WireError;
use super::azure::{azure_headers, azure_jobs_url, azure_query, azure_version};
use super::insert_header;

pub(super) const JSON_CONTENT: &str = "application/json";
const DEFAULT_DEADLINE: Duration = Duration::from_secs(600);

// One fine-tuning call the native side can send, whichever provider or operation named it.
#[derive(Clone, Copy)]
pub(crate) enum WireCall<'call> {
    OpenAiCreate(&'call OpenAiCreateCall),
    AzureCreate(&'call AzureCreateCall),
    OpenAiRead(&'call OpenAiReadCall<'call>),
    AzureRead(&'call AzureReadCall<'call>),
    OpenAiCancel(&'call OpenAiFineTuningCancel<'call>),
    AzureCancel(&'call AzureFineTuningCancel<'call>),
}

// The create target's two supported providers; the vertex gap stays the caller's to answer.
pub(crate) const fn create_call(call: &FineTuningCreateTarget) -> Result<WireCall<'_>, WireError> {
    match call {
        FineTuningCreateTarget::OpenAi(inner) => Ok(WireCall::OpenAiCreate(inner)),
        FineTuningCreateTarget::Azure(inner) => Ok(WireCall::AzureCreate(inner)),
        FineTuningCreateTarget::Vertex(_) => Err(WireError::Vertex),
    }
}

pub(crate) const fn read_call<'call>(call: &'call ReadTarget<'call>) -> WireCall<'call> {
    match call {
        ReadTarget::OpenAi(inner) => WireCall::OpenAiRead(inner),
        ReadTarget::Azure(inner) => WireCall::AzureRead(inner),
    }
}

pub(crate) const fn cancel_call<'call>(
    call: &'call FineTuningCancelTarget<'call>,
) -> WireCall<'call> {
    match call {
        FineTuningCancelTarget::OpenAi(inner) => WireCall::OpenAiCancel(inner),
        FineTuningCancelTarget::Azure(inner) => WireCall::AzureCancel(inner),
    }
}

type OutboundResult = Result<OutboundRequest, WireError>;

// The one request builder: every operation and provider fills the same outbound shape.
pub(crate) fn request_for(
    call: WireCall<'_>,
    integrations: Option<&[Json]>,
) -> Result<(OutboundRequest, bool), WireError> {
    match call {
        WireCall::OpenAiCreate(inner) => Ok((openai_create(inner, integrations)?, false)),
        WireCall::AzureCreate(inner) => Ok((azure_create(inner, integrations)?, true)),
        WireCall::OpenAiRead(inner) => Ok((openai_read(inner)?, false)),
        WireCall::AzureRead(inner) => Ok((azure_read(inner)?, true)),
        WireCall::OpenAiCancel(inner) => Ok((openai_cancel(inner)?, false)),
        WireCall::AzureCancel(inner) => Ok((azure_cancel(inner)?, true)),
    }
}

fn openai_create(call: &OpenAiCreateCall, integrations: Option<&[Json]>) -> OutboundResult {
    Ok(OutboundRequest {
        method: Method::POST,
        url: openai_jobs_url(&call.api_base, "")?,
        query: Vec::new(),
        headers: openai_headers(call.api_key.as_ref(), call.organization.as_deref(), true)?,
        body: RequestBody::Json(create_body(&call.data, false, integrations)?),
        deadline: deadline(&call.timeout),
    })
}

fn azure_create(call: &AzureCreateCall, integrations: Option<&[Json]>) -> OutboundResult {
    let (key, api_base, version) = azure_api(
        call.api_key.as_ref(),
        call.api_base.as_deref(),
        call.api_version.as_deref(),
    )?;
    let mut headers = azure_headers(key, version)?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    Ok(OutboundRequest {
        method: Method::POST,
        url: azure_jobs_url(api_base, version, "")?,
        query: azure_query(version),
        headers,
        body: RequestBody::Json(create_body(&call.data, true, integrations)?),
        deadline: deadline(&call.timeout),
    })
}

fn openai_read(call: &OpenAiReadCall<'_>) -> OutboundResult {
    let operation = call.values.operation;
    let headers = openai_headers(call.api_key, call.organization, false)?;
    let url = openai_jobs_url(&call.api_base, &read_suffix(operation)?)?;
    let query = paging(operation);
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query,
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

fn azure_read(call: &AzureReadCall<'_>) -> OutboundResult {
    let (key, api_base, version) = azure_api(call.api_key, call.api_base, call.api_version)?;
    let mut query = azure_query(version);
    query.extend(paging(call.values.operation));
    let url = azure_jobs_url(api_base, version, &read_suffix(call.values.operation)?)?;
    let headers = azure_headers(key, version)?;
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query,
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

fn openai_cancel(call: &OpenAiFineTuningCancel<'_>) -> OutboundResult {
    let headers = openai_headers(call.api_key, call.organization, false)?;
    let suffix = format!("/{}/cancel", encode_segment(call.values.job_id, "job_id")?);
    let url = openai_jobs_url(&call.api_base, &suffix)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

fn azure_cancel(call: &AzureFineTuningCancel<'_>) -> OutboundResult {
    let (key, api_base, version) = azure_api(call.api_key, call.api_base, call.api_version)?;
    let suffix = format!("/{}/cancel", encode_segment(call.values.job_id, "job_id")?);
    let url = azure_jobs_url(api_base, version, &suffix)?;
    let headers = azure_headers(key, version)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: azure_query(version),
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

// The openai client's own headers: the bearer token, the organization, and JSON on a body-carrying call.
fn openai_headers(
    key: Option<&SecretString>,
    organization: Option<&str>,
    is_json: bool,
) -> Result<HeaderMap, WireError> {
    let Some(found_key) = key else {
        return Err(WireError::MissingOpenAiKey);
    };
    let mut headers = HeaderMap::new();
    if is_json {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    }
    insert_header(&mut headers, "accept", JSON_CONTENT)?;
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {}", found_key.expose_secret()),
    )?;
    if let Some(name) = organization {
        insert_header(&mut headers, "openai-organization", name)?;
    }
    Ok(headers)
}

// The two parts every Azure call needs, and the version request_for then reads.
fn azure_api<'call>(
    key: Option<&'call SecretString>,
    api_base: Option<&'call str>,
    api_version: Option<&'call str>,
) -> Result<(&'call SecretString, &'call str, &'call str), WireError> {
    let Some(found_key) = key else {
        return Err(WireError::MissingAzureKey);
    };
    let Some(found_base) = api_base else {
        return Err(WireError::MissingAzureBase);
    };
    Ok((found_key, found_base, azure_version(api_version)))
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

fn read_suffix(operation: ReadOperation<'_>) -> Result<String, WireError> {
    match operation {
        ReadOperation::List { .. } => Ok(String::new()),
        ReadOperation::Retrieve { job_id } => Ok(format!("/{}", encode_segment(job_id, "job_id")?)),
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
