use std::collections::BTreeMap;

use reqwest::multipart::{Form, Part};
use reqwest::{RequestBuilder, Response};
use secrecy::ExposeSecret as _;
use serde::de::DeserializeOwned;
use tokio::time::sleep;

use super::FileSanitizer;
use super::error::FileSanitizationError;
use super::log::{log_api_request, log_api_response};
use super::wire::{SanitizeResult, StatusResponse, UploadResponse};

const QUEUED_STATUSES: [&str; 2] = ["created", "in progress"];

fn request_headers(
    sanitizer: &FileSanitizer,
    user_api_key_alias: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([(
        "APP-ID".to_owned(),
        sanitizer.settings.api_key.expose_secret().to_owned(),
    )]);
    if let Some(alias) = user_api_key_alias.filter(|alias| !alias.is_empty()) {
        headers.insert("X-LiteLLM-Key-Alias".to_owned(), alias.to_owned());
    }
    headers
}

fn with_headers(request: RequestBuilder, headers: &BTreeMap<String, String>) -> RequestBuilder {
    headers.iter().fold(request, |builder, (name, value)| {
        builder.header(name, value)
    })
}

pub(super) async fn upload_and_poll(
    sanitizer: &FileSanitizer,
    file_data: &[u8],
    filename: &str,
    user_api_key_alias: Option<&str>,
) -> Result<SanitizeResult, FileSanitizationError> {
    let url = format!("{}/api/sanitizeFile", sanitizer.settings.api_base);
    let headers = request_headers(sanitizer, user_api_key_alias);
    log_api_request("POST", &url, &headers, &format!("file upload: {filename}"));

    let mime = mime_guess::from_path(filename).first_or_octet_stream();
    let part = Part::bytes(file_data.to_vec())
        .file_name(filename.to_owned())
        .mime_str(mime.as_ref())
        .map_err(|source| FileSanitizationError::UploadMime { source })?;
    let upload = with_headers(sanitizer.client.post(&url), &headers)
        .multipart(Form::new().part("file", part))
        .timeout(sanitizer.settings.request_timeout);
    let response = upload.send().await.map_err(request_error)?;
    let status = response.status();
    let uploaded: UploadResponse =
        read_json(response, sanitizer.settings.max_response_bytes).await?;
    let returned = uploaded.job_id.filter(|job_id| !job_id.is_empty());
    log_api_response(&url, status.as_u16(), &format!("jobId={returned:?}"));
    let job_id = returned.ok_or(FileSanitizationError::MissingJobId)?;
    tracing::debug!(
        job_id,
        "Prompt Security Guardrail: File sanitization started"
    );
    poll(sanitizer, &url, &headers, &job_id).await
}

async fn poll(
    sanitizer: &FileSanitizer,
    url: &str,
    headers: &BTreeMap<String, String>,
    job_id: &str,
) -> Result<SanitizeResult, FileSanitizationError> {
    let attempts = sanitizer.settings.max_poll_attempts;
    for attempt in 1..=attempts {
        sleep(sanitizer.settings.poll_interval).await;
        let polled = poll_once(sanitizer, url, headers, job_id).await?;
        match polled.status.as_deref() {
            Some("done") => {
                tracing::debug!(
                    job_id,
                    "Prompt Security Guardrail: File sanitization completed"
                );
                return Ok(SanitizeResult::from(polled));
            }
            Some(queued) if QUEUED_STATUSES.contains(&queued) => {
                tracing::debug!(
                    queued,
                    job_id,
                    attempt,
                    attempts,
                    "Prompt Security Guardrail: File sanitization still queued",
                );
            }
            other => {
                return Err(FileSanitizationError::UnexpectedStatus {
                    status: other.unwrap_or("None").to_owned(),
                });
            }
        }
    }
    Err(FileSanitizationError::Timeout)
}

async fn poll_once(
    sanitizer: &FileSanitizer,
    url: &str,
    headers: &BTreeMap<String, String>,
    job_id: &str,
) -> Result<StatusResponse, FileSanitizationError> {
    log_api_request("GET", url, headers, &format!("jobId={job_id}"));
    let request = with_headers(sanitizer.client.get(url), headers)
        .query(&[("jobId", job_id)])
        .timeout(sanitizer.settings.request_timeout);
    let response = request.send().await.map_err(request_error)?;
    let status_code = response.status().as_u16();
    let polled: StatusResponse = read_json(response, sanitizer.settings.max_response_bytes).await?;
    log_api_response(
        url,
        status_code,
        &format!("jobId={job_id} status={:?}", polled.status),
    );
    Ok(polled)
}

fn request_error(source: reqwest::Error) -> FileSanitizationError {
    if source.is_timeout() {
        FileSanitizationError::RequestTimeout { source }
    } else {
        FileSanitizationError::Request { source }
    }
}

async fn read_json<T>(mut response: Response, limit: usize) -> Result<T, FileSanitizationError>
where
    T: DeserializeOwned,
{
    let status = response.status();
    if !status.is_success() {
        return Err(FileSanitizationError::HttpStatus { status });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(FileSanitizationError::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|source| FileSanitizationError::ResponseJson { source })
}
