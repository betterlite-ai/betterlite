use std::time::Duration;

use betterlite_azure::fine_tuning::ensure_training_type;
use betterlite_core::api::fine_tuning_cancel_call::{
    AzureFineTuningCancel, OpenAiFineTuningCancel,
};
use betterlite_core::api::fine_tuning_create::{AzureCreateCall, OpenAiCreateCall};
use betterlite_core::api::fine_tuning_read::{AzureReadCall, OpenAiReadCall, ReadOperation};
use betterlite_core::api::resource_sdk_settings::timeout::FineTuningTimeout;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use super::super::error::WireError;
use super::azure::{azure_headers, azure_jobs_url, azure_query, azure_version};
use super::insert_header;

const JSON_CONTENT: &str = "application/json";
const DEFAULT_DEADLINE: Duration = Duration::from_secs(600);

pub(super) fn openai_create_request(
    call: &OpenAiCreateCall,
    integrations: Option<&[Json]>,
) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key.as_ref() else {
        return Err(WireError::MissingOpenAiKey);
    };
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT));
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {}", key.expose_secret()),
    )?;
    if let Some(organization) = call.organization.as_deref() {
        insert_header(&mut headers, "openai-organization", organization)?;
    }
    let url = Url::parse(&format!(
        "{}/fine_tuning/jobs",
        call.api_base.trim_end_matches('/')
    ))
    .map_err(|source| WireError::Url { source })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(create_body(&call.data, false, integrations)?),
        deadline: deadline(&call.timeout),
    })
}

pub(super) fn azure_create_request(
    call: &AzureCreateCall,
    integrations: Option<&[Json]>,
) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key.as_ref() else {
        return Err(WireError::MissingAzureKey);
    };
    let Some(api_base) = call.api_base.as_deref() else {
        return Err(WireError::MissingAzureBase);
    };
    let version = azure_version(call.api_version.as_deref());
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

pub(super) fn openai_read_request(call: &OpenAiReadCall<'_>) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key else {
        return Err(WireError::MissingOpenAiKey);
    };
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {}", key.expose_secret()),
    )?;
    if let Some(organization) = call.organization {
        insert_header(&mut headers, "openai-organization", organization)?;
    }
    let url = Url::parse(&format!(
        "{}/fine_tuning/jobs{}",
        call.api_base.trim_end_matches('/'),
        read_suffix(call.values.operation)
    ))
    .map_err(|source| WireError::Url { source })?;
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query: paging(call.values.operation),
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

pub(super) fn azure_read_request(call: &AzureReadCall<'_>) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key else {
        return Err(WireError::MissingAzureKey);
    };
    let Some(api_base) = call.api_base else {
        return Err(WireError::MissingAzureBase);
    };
    let version = azure_version(call.api_version);
    let url = azure_jobs_url(api_base, version, &read_suffix(call.values.operation))?;
    let mut query = azure_query(version);
    query.extend(paging(call.values.operation));
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query,
        headers: azure_headers(key, version)?,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

pub(super) fn openai_cancel_request(
    call: &OpenAiFineTuningCancel<'_>,
) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key else {
        return Err(WireError::MissingOpenAiKey);
    };
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {}", key.expose_secret()),
    )?;
    if let Some(organization) = call.organization {
        insert_header(&mut headers, "openai-organization", organization)?;
    }
    let url = Url::parse(&format!(
        "{}/fine_tuning/jobs/{}/cancel",
        call.api_base.trim_end_matches('/'),
        call.values.job_id
    ))
    .map_err(|source| WireError::Url { source })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
}

pub(super) fn azure_cancel_request(
    call: &AzureFineTuningCancel<'_>,
) -> Result<OutboundRequest, WireError> {
    let Some(key) = call.api_key else {
        return Err(WireError::MissingAzureKey);
    };
    let Some(api_base) = call.api_base else {
        return Err(WireError::MissingAzureBase);
    };
    let version = azure_version(call.api_version);
    let suffix = format!("/{}/cancel", call.values.job_id);
    Ok(OutboundRequest {
        method: Method::POST,
        url: azure_jobs_url(api_base, version, &suffix)?,
        query: azure_query(version),
        headers: azure_headers(key, version)?,
        body: RequestBody::Empty,
        deadline: deadline(call.values.timeout),
    })
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
