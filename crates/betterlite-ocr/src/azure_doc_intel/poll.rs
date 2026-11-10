use std::time::Duration;

use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::header::RETRY_AFTER;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use tokio::time::{Instant, sleep, timeout_at};
use url::Url;

use super::AZURE_DI_SUBSCRIPTION_HEADER;
use super::normalize;
use super::wire::{AnalyzeOperation, KnownStatus, OperationStatus};
use crate::ocr_config::decode::decode_response;
use crate::ocr_config::{DecodedOcrResponse, OCR_POLL_RETRY_SECS, OcrResponseContext};
use crate::ocr_error::{OcrError, OcrHeaderError, OcrTransportError};
use crate::ocr_http::read_response_bytes;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

const OPERATION_LOCATION: &str = "operation-location";
const POLL_HEADERS: [&str; 2] = [AZURE_DI_SUBSCRIPTION_HEADER, "authorization"];

pub(super) async fn transform_ocr_response(
    model: &str,
    response: StreamResponse,
    context: OcrResponseContext<'_>,
) -> Result<OCRResponse, OcrError> {
    let decoded = read_operation(response, context).await?;
    let mut normalized = normalize::normalize(
        model,
        decoded.data,
        context.connection.settings.doc_intelligence_dpi,
    )?;
    normalized.provider_native_response = decoded.native;
    Ok(normalized)
}

async fn read_operation(
    response: StreamResponse,
    context: OcrResponseContext<'_>,
) -> Result<DecodedOcrResponse<AnalyzeOperation>, OcrError> {
    let native = context.request_format == RequestFormat::Native;
    if response.response.status() != StatusCode::ACCEPTED {
        let bytes = read_response_bytes(response, context.connection.max_response_bytes).await?;
        context.hooks.response_received(&bytes).await?;
        return decode_response(&bytes, native);
    }
    let location = response
        .response
        .headers()
        .get(OPERATION_LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(OcrError::PollLocation)?
        .to_owned();
    let submission = Url::parse(context.url).map_err(|_source| OcrError::PollOrigin)?;
    let operation = Url::parse(&location).map_err(|_source| OcrError::PollOrigin)?;
    if submission.origin() != operation.origin()
        || !operation.username().is_empty()
        || operation.password().is_some()
    {
        return Err(OcrError::PollOrigin);
    }
    let accepted = read_response_bytes(response, context.connection.max_response_bytes).await?;
    context.hooks.response_received(&accepted).await?;
    poll_operation(operation, native, context).await
}

enum PollOutcome {
    Finished(Box<DecodedOcrResponse<AnalyzeOperation>>),
    Wait(u64),
}

async fn poll_operation(
    url: Url,
    native: bool,
    context: OcrResponseContext<'_>,
) -> Result<DecodedOcrResponse<AnalyzeOperation>, OcrError> {
    let deadline = Instant::now()
        .checked_add(context.connection.settings.poll_timeout)
        .ok_or(OcrError::PollTimeout)?;
    loop {
        match poll_once(url.clone(), native, deadline, context).await? {
            PollOutcome::Finished(decoded) => return Ok(*decoded),
            PollOutcome::Wait(retry) => {
                timeout_at(deadline, sleep(Duration::from_secs(retry)))
                    .await
                    .map_err(|_elapsed| OcrError::PollTimeout)?;
            }
        }
    }
}

async fn poll_once(
    url: Url,
    native: bool,
    deadline: Instant,
    context: OcrResponseContext<'_>,
) -> Result<PollOutcome, OcrError> {
    let response = poll_answer(url, deadline, context).await?;
    let retry = response
        .response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(OCR_POLL_RETRY_SECS)
        .max(1);
    let decoded = read_poll_answer(
        response,
        native,
        deadline,
        context.connection.max_response_bytes,
    )
    .await?;
    match &decoded.data.status {
        Some(OperationStatus::Known(KnownStatus::Succeeded)) => {
            context
                .hooks
                .response_received(decoded.text.as_bytes())
                .await?;
            Ok(PollOutcome::Finished(Box::new(decoded)))
        }
        Some(OperationStatus::Known(KnownStatus::Running | KnownStatus::NotStarted)) => {
            Ok(PollOutcome::Wait(retry))
        }
        status => Err(OcrError::OperationStatus(
            status
                .as_ref()
                .map_or_else(|| "None".to_owned(), |state| state.text().to_owned()),
        )),
    }
}

async fn poll_answer(
    url: Url,
    deadline: Instant,
    context: OcrResponseContext<'_>,
) -> Result<StreamResponse, OcrError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(OcrError::PollTimeout)?;
    let request = polling_request(
        url,
        context.headers,
        remaining.min(context.connection.timeout),
    )?;
    timeout_at(deadline, context.client.polling_http.send(request))
        .await
        .map_err(|_elapsed| OcrError::PollTimeout)?
        .map_err(transport_failure)
}

async fn read_poll_answer(
    response: StreamResponse,
    native: bool,
    deadline: Instant,
    limit: usize,
) -> Result<DecodedOcrResponse<AnalyzeOperation>, OcrError> {
    timeout_at(deadline, async {
        let bytes = read_response_bytes(response, limit).await?;
        decode_response(&bytes, native)
    })
    .await
    .map_err(|_elapsed| OcrError::PollTimeout)?
}

fn polling_request(
    url: Url,
    headers: &[(String, String)],
    timeout: Duration,
) -> Result<OutboundRequest, OcrError> {
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: polling_headers(headers)?,
        body: RequestBody::Empty,
        deadline: timeout,
    })
}

fn polling_headers(headers: &[(String, String)]) -> Result<HeaderMap, OcrError> {
    let mut allowed = HeaderMap::new();
    for (name, value) in headers {
        if !POLL_HEADERS
            .iter()
            .any(|permitted| name.eq_ignore_ascii_case(permitted))
        {
            continue;
        }
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Name {
                name: name.clone(),
                source,
            })
        })?;
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Value {
                name: name.clone(),
                source,
            })
        })?;
        allowed.append(header_name, header_value);
    }
    Ok(allowed)
}

fn transport_failure(error: TransportError) -> OcrError {
    if matches!(
        error,
        TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
    ) {
        return OcrError::Transport(OcrTransportError::Timeout);
    }
    OcrError::Transport(OcrTransportError::Failed(error))
}
