use betterlite_config::sections::xai_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::url_path::encode_segment;
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::Map;
use speedate::{Date, DateTime};

use crate::api::batch_records::CreateBatchRequest;
use crate::batches::records::{
    BatchEndpoint, BatchFailure, BatchFailures, BatchObject, BatchObjectKind, BatchStatus,
    RequestCounts,
};
use crate::files::stream::flatten_headers;
use crate::xai_identity::{KeyOrder, get_api_key};

use super::error::XaiBatchesError;
use super::records::{XaiBatch, XaiBatchList, XaiCreateBatchRequest};

pub const XAI_API_BASE: &str = "https://api.x.ai/v1";
pub const XAI_RESULTS_PAGE_SIZE: i64 = 1000;
pub const DEFAULT_BATCH_NAME: &str = "litellm-batch";
pub const DEFAULT_BATCH_ENDPOINT: &str = "/v1/chat/completions";
pub const JSONL_CONTENT_TYPE: &str = "application/jsonl";
const COMPLETION_WINDOW: &str = "24h";

#[must_use]
pub fn get_xai_api_base(api_base: Option<&str>, settings: &Settings) -> String {
    let resolved = api_base
        .or(settings.api_base.as_deref())
        .unwrap_or(XAI_API_BASE)
        .trim_end_matches('/');
    resolved.strip_suffix("/v1").unwrap_or(resolved).to_owned()
}

pub fn get_xai_auth_headers(
    mut headers: HeaderMap,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<HeaderMap, XaiBatchesError> {
    let key =
        get_api_key(api_key, KeyOrder::Chat, settings).ok_or(XaiBatchesError::MissingApiKey)?;
    let value = HeaderValue::try_from(format!("Bearer {}", key.expose_secret()))
        .map_err(|source| XaiBatchesError::Authorization { source })?;
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

pub fn xai_batches_url(
    api_base: Option<&str>,
    batch_id: Option<&str>,
    suffix: &str,
    settings: &Settings,
) -> Result<String, XaiBatchesError> {
    let base = format!("{}/v1/batches", get_xai_api_base(api_base, settings));
    let Some(id) = batch_id else {
        return Ok(base);
    };
    let encoded =
        encode_segment(id, "batch_id").map_err(|source| XaiBatchesError::Path { source })?;
    Ok(format!("{base}/{encoded}{suffix}"))
}

pub fn to_create_batch_body(
    data: &CreateBatchRequest,
) -> Result<XaiCreateBatchRequest, XaiBatchesError> {
    let input_file_id = data
        .input_file_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or(XaiBatchesError::InputFileRequired)?;
    let name = data
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("name"))
        .filter(|name| !name.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_BATCH_NAME.to_owned());
    Ok(XaiCreateBatchRequest {
        name,
        input_file_id,
    })
}

#[must_use]
pub const fn create_endpoint(data: &CreateBatchRequest) -> &'static str {
    match data.endpoint {
        Some(endpoint) => endpoint_wire_name(endpoint),
        None => DEFAULT_BATCH_ENDPOINT,
    }
}

const fn endpoint_wire_name(endpoint: BatchEndpoint) -> &'static str {
    match endpoint {
        BatchEndpoint::ChatCompletions => "/v1/chat/completions",
        BatchEndpoint::Embeddings => "/v1/embeddings",
        BatchEndpoint::Completions => "/v1/completions",
        BatchEndpoint::Responses => "/v1/responses",
        BatchEndpoint::Ocr => "/v1/ocr",
        BatchEndpoint::ImageGenerations => "/v1/images/generations",
        BatchEndpoint::ImageEdits => "/v1/images/edits",
        BatchEndpoint::VideoGenerations => "/v1/videos/generations",
        BatchEndpoint::Videos => "/v1/videos",
        BatchEndpoint::VideoEdits => "/v1/videos/edits",
        BatchEndpoint::VideoExtensions => "/v1/videos/extensions",
    }
}

#[must_use]
pub fn xai_batch_status(batch: &XaiBatch) -> BatchStatus {
    if batch.cancel_time.is_some() {
        let failed = batch
            .cancel_by_xai_message
            .as_deref()
            .is_some_and(|text| !text.is_empty());
        return if failed {
            BatchStatus::Failed
        } else {
            BatchStatus::Cancelled
        };
    }
    if batch.state.num_requests == 0 {
        return BatchStatus::Validating;
    }
    if batch.state.num_pending > 0 {
        return BatchStatus::InProgress;
    }
    BatchStatus::Completed
}

fn batch_errors(message: Option<&str>) -> Option<BatchFailures> {
    let text = message?;
    Some(BatchFailures {
        data: Some(vec![BatchFailure {
            code: None,
            line: None,
            message: Some(text.to_owned()),
            param: None,
        }]),
        object: Some("list".to_owned()),
    })
}

fn batch_metadata(name: &str) -> Option<IndexMap<String, String>> {
    if name.is_empty() {
        return None;
    }
    Some(IndexMap::from([("name".to_owned(), name.to_owned())]))
}

#[must_use]
pub fn to_litellm_batch(batch: &XaiBatch, endpoint: &str) -> BatchObject {
    let status = xai_batch_status(batch);
    let cancelled_at = to_unix_timestamp(batch.cancel_time.as_deref());
    let message = batch
        .cancel_by_xai_message
        .as_deref()
        .filter(|text| !text.is_empty());
    let cancelled = status == BatchStatus::Cancelled;
    let failed = status == BatchStatus::Failed;
    BatchObject {
        id: batch.batch_id.clone(),
        completion_window: COMPLETION_WINDOW.to_owned(),
        created_at: to_unix_timestamp(batch.create_time.as_deref()).unwrap_or(0),
        endpoint: endpoint.to_owned(),
        input_file_id: batch.input_file_id.clone().unwrap_or_default(),
        object: BatchObjectKind::Batch,
        status,
        cancelled_at: cancelled.then_some(cancelled_at).flatten(),
        cancelling_at: None,
        completed_at: None,
        error_file_id: None,
        errors: batch_errors(message),
        expired_at: None,
        expires_at: to_unix_timestamp(batch.expire_time.as_deref()),
        failed_at: failed.then_some(cancelled_at).flatten(),
        finalizing_at: None,
        in_progress_at: None,
        metadata: batch_metadata(&batch.name),
        model: None,
        output_file_id: (status == BatchStatus::Completed).then(|| batch.batch_id.clone()),
        request_counts: Some(RequestCounts {
            completed: batch.state.num_success,
            failed: batch
                .state
                .num_error
                .saturating_add(batch.state.num_cancelled),
            total: batch.state.num_requests,
        }),
        usage: None,
        provider_fields: Map::new(),
    }
}

// xAI returns RFC 3339 over gRPC but a bare YYYY-MM-DD over REST; anything else is no timestamp
fn to_unix_timestamp(value: Option<&str>) -> Option<i64> {
    let text = value?;
    if let Ok(moment) = DateTime::parse_str_rfc3339(text) {
        return Some(moment.timestamp_tz());
    }
    Date::parse_str(text).ok().map(|date| date.timestamp())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchListObject {
    List,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiBatchListResponse {
    pub object: BatchListObject,
    pub data: Vec<BatchObject>,
    pub first_id: Option<String>,
    pub last_id: Option<String>,
    pub has_more: bool,
    pub next_page_token: Option<String>,
}

#[must_use]
pub fn to_openai_batch_list(page: &XaiBatchList) -> OpenAiBatchListResponse {
    let data: Vec<BatchObject> = page
        .batches
        .iter()
        .map(|batch| to_litellm_batch(batch, DEFAULT_BATCH_ENDPOINT))
        .collect();
    let token = page
        .pagination_token
        .clone()
        .filter(|token| !token.is_empty());
    OpenAiBatchListResponse {
        object: BatchListObject::List,
        has_more: token.is_some(),
        next_page_token: token,
        first_id: data.first().map(|batch| batch.id.clone()),
        last_id: data.last().map(|batch| batch.id.clone()),
        data,
    }
}

// after=None omits the cursor, an explicit empty one is sent, and a falsey limit asks for the page size
#[must_use]
pub fn results_params(after: Option<&str>, limit: Option<i64>) -> Vec<(String, String)> {
    let page_size = limit
        .filter(|size| *size != 0)
        .unwrap_or(XAI_RESULTS_PAGE_SIZE);
    let mut params = vec![("limit".to_owned(), page_size.to_string())];
    if let Some(token) = after {
        params.push(("pagination_token".to_owned(), token.to_owned()));
    }
    params
}

pub fn raise_for_xai_status(response: &UpstreamResponse) -> Result<(), XaiBatchesError> {
    if response.status < 400 {
        return Ok(());
    }
    let message = str::from_utf8(&response.body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", response.body.len()),
        str::to_owned,
    );
    Err(XaiBatchesError::Status {
        status: response.status,
        message,
        headers: flatten_headers(&response.headers),
    })
}
