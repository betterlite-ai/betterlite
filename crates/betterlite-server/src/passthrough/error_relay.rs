use async_trait::async_trait;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_http::error::HttpError;
// ast-grep-ignore: no-long-names
use betterlite_http::passthrough::error_body::PASSTHROUGH_UPSTREAM_ERROR_BODY_MAX_LOG_CHARS as MAX_LOG_CHARS;
use betterlite_http::passthrough::error_body::{ChunkStream, error_body_preview_and_relay};
use betterlite_logging::langfuse_assembly::REDACTED_BY_LITELLM;
use betterlite_logging::spend_bodies::redaction::should_redact_message_logging;
use betterlite_logging::spend_bodies::support::Fields;
use futures_util::{Stream, StreamExt as _};
use http::{HeaderMap, StatusCode};
use icu_properties::CodePointMapData;
use icu_properties::props::GeneralCategory;
use thiserror::Error;
use url::Url;

use super::request_context::{PassthroughLoggingObj, as_object};
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError};

// ProxyLogging.post_call_failure_hook, reduced to what the pass-through failure paths report with
#[async_trait]
pub trait PassthroughFailureHook: Send + Sync {
    async fn post_call_failure_hook(
        &self,
        user_api_key_dict: &PrincipalFacts,
        original_exception: &ProxyHookError,
        request_data: &mut RequestData,
    ) -> Result<(), PassthroughHookError>;
}

#[derive(Debug, Error)]
#[error("the post-call failure hook failed")]
pub struct PassthroughHookError;

// should_redact_message_logging over the logging object's details, with the proxy-wide setting
pub trait MessageRedaction: Send + Sync {
    fn should_redact(&self, model_call_details: &Fields) -> bool;
}

pub struct LoggingRedaction {
    pub global: bool,
}

impl MessageRedaction for LoggingRedaction {
    fn should_redact(&self, model_call_details: &Fields) -> bool {
        let litellm_params = model_call_details.get("litellm_params").and_then(as_object);
        let dynamic_params = model_call_details
            .get("standard_callback_dynamic_params")
            .and_then(as_object);
        should_redact_message_logging(litellm_params, dynamic_params, self.global)
    }
}

// the upstream response a failure path relays, with the payload and ports it reports through
pub struct RelayResponse {
    pub status: StatusCode,
    pub method: String,
    pub url: String,
    pub headers: HeaderMap,
    pub body: ChunkStream,
}

pub struct FailureReporting<'call> {
    pub user_api_key_dict: &'call PrincipalFacts,
    pub request_payload: RequestData,
    pub logging_obj: &'call PassthroughLoggingObj,
    pub hook: &'call dyn PassthroughFailureHook,
    pub redaction: &'call dyn MessageRedaction,
}

// _truncate_upstream_error_body: at most the log budget in characters, then a note saying it was cut
#[must_use]
pub fn truncate_upstream_error_body(body: &str) -> String {
    if body.chars().count() <= MAX_LOG_CHARS {
        return body.to_owned();
    }
    let kept: String = body.chars().take(MAX_LOG_CHARS).collect();
    format!("{kept}... (truncated at {MAX_LOG_CHARS} chars)")
}

// _sanitize_upstream_error_body: non-printable code points become spaces and whitespace runs collapse
#[must_use]
pub fn sanitize_upstream_error_body(body: &str) -> String {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let spaced: String = body
        .chars()
        .map(|character| {
            // Python's str.isprintable: space and every category but the Cc, Cf, Cs, Co, Cn, Zl, Zp and Zs ones
            let printable = character == ' '
                || !matches!(
                    categories.get(character),
                    GeneralCategory::Control
                        | GeneralCategory::Format
                        | GeneralCategory::Surrogate
                        | GeneralCategory::PrivateUse
                        | GeneralCategory::Unassigned
                        | GeneralCategory::LineSeparator
                        | GeneralCategory::ParagraphSeparator
                        | GeneralCategory::SpaceSeparator
                );
            if printable { character } else { ' ' }
        })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

// what the log line and the exception detail carry: the sanitized preview, or the redaction marker
fn failed_body_preview(
    redaction: &dyn MessageRedaction,
    logging_obj: &PassthroughLoggingObj,
    preview: &str,
) -> String {
    if redaction.should_redact(&logging_obj.model_call_details) {
        return REDACTED_BY_LITELLM.to_owned();
    }
    truncate_upstream_error_body(&sanitize_upstream_error_body(preview))
}

async fn report_upstream_failure(
    mut reporting: FailureReporting<'_>,
    status: StatusCode,
    method: &str,
    url: &str,
    upstream_error_body: &str,
) {
    tracing::warn!(
        "pass_through_endpoint: upstream {method} {} returned {}: {upstream_error_body}",
        without_query(url),
        status.as_u16()
    );
    let exception = HttpException {
        status: status.as_u16(),
        detail: format!(
            "Upstream passthrough request failed with status {}: {upstream_error_body}",
            status.as_u16()
        ),
    };
    let error = ProxyHookError::from(exception);
    let hook_result = reporting
        .hook
        .post_call_failure_hook(
            reporting.user_api_key_dict,
            &error,
            &mut reporting.request_payload,
        )
        .await;
    if let Err(hook_error) = hook_result {
        tracing::warn!(
            "pass_through_endpoint: post_call_failure_hook raised for upstream error: {hook_error}"
        );
    }
}

// _log_passthrough_upstream_failure: log the upstream error, report it, and relay its body unchanged
pub async fn log_upstream_failure(
    response: RelayResponse,
    reporting: FailureReporting<'_>,
) -> RelayResponse {
    if response.status.as_u16() < 400 {
        return response;
    }
    let status = response.status;
    let method = response.method;
    let url = response.url;
    let relay_body = error_body_preview_and_relay(status, &response.headers, response.body).await;
    let upstream_error_body = failed_body_preview(
        reporting.redaction,
        reporting.logging_obj,
        &relay_body.preview,
    );
    report_upstream_failure(reporting, status, &method, &url, &upstream_error_body).await;
    RelayResponse {
        status,
        method,
        url,
        headers: relay_body.headers,
        body: relay_body.stream.boxed(),
    }
}

// _relay_reporting_failures: chunks pass through, and a mid-stream error is reported once unless upstream already failed
pub fn relay_reporting_failures(
    mut stream: ChunkStream,
    upstream_status: u16,
    mut reporting: FailureReporting<'_>,
) -> impl Stream<Item = Result<Vec<u8>, HttpError>> + Send + '_ {
    async_stream::stream! {
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => yield Ok(bytes),
                Err(error) => {
                    if upstream_status < 400 {
                        let original = ProxyHookError::Other(error.to_string());
                        let hook_result = reporting
                            .hook
                            .post_call_failure_hook(
                                reporting.user_api_key_dict,
                                &original,
                                &mut reporting.request_payload,
                            )
                            .await;
                        if let Err(hook_error) = hook_result {
                            tracing::warn!(
                                "pass_through_endpoint: post_call_failure_hook raised for a mid-stream upstream error: {hook_error}"
                            );
                        }
                    }
                    yield Err(error);
                    break;
                }
            }
        }
    }
}

// response.url.copy_with(query=None, fragment=None): the log line never carries the query or fragment
pub(super) fn without_query(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}
