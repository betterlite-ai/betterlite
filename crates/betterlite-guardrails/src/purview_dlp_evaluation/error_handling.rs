use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use reqwest::StatusCode;
use thiserror::Error;

use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::wire::Activity;

pub const POLICY_BLOCK_STATUS: u16 = 400;
const BAD_GATEWAY_STATUS: u16 = 502;
const UPSTREAM_ERROR: &str = "Microsoft Purview DLP: upstream policy evaluation failed";
const BLOCKED_ERROR: &str = "Microsoft Purview DLP: Content blocked by policy";

#[derive(Debug, Clone, PartialEq, Error)]
#[error("Microsoft Purview DLP: request failed with status {status}")]
pub struct PurviewDlpHttpError {
    pub status: u16,
    pub detail: IndexMap<String, GuardrailValue>,
    pub retry_after: Option<String>,
}

fn detail(entries: &[(&str, GuardrailValue)]) -> IndexMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[must_use]
pub const fn activity_name(activity: Activity) -> &'static str {
    match activity {
        Activity::UploadText => "uploadText",
        Activity::DownloadText => "downloadText",
    }
}

fn failure_detail(
    activity: Activity,
    upstream_status: Option<u16>,
    exception: &str,
) -> IndexMap<String, GuardrailValue> {
    let mut entries = vec![
        ("error", GuardrailValue::Text(UPSTREAM_ERROR.to_owned())),
        (
            "activity",
            GuardrailValue::Text(activity_name(activity).to_owned()),
        ),
    ];
    if let Some(status) = upstream_status {
        entries.push((
            "upstream_status",
            GuardrailValue::UnsignedInteger(u64::from(status)),
        ));
    }
    entries.push(("exception", GuardrailValue::Text(exception.to_owned())));
    detail(&entries)
}

fn upstream_http_error(
    status: StatusCode,
    retry_after: Option<&str>,
    activity: Activity,
    error: &PurviewGraphError,
) -> PurviewDlpHttpError {
    let upstream_status = status.as_u16();
    let client_status = if matches!(upstream_status, 401 | 403) {
        BAD_GATEWAY_STATUS
    } else {
        upstream_status
    };
    PurviewDlpHttpError {
        status: client_status,
        detail: failure_detail(activity, Some(upstream_status), &error.to_string()),
        retry_after: retry_after
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    }
}

#[must_use]
pub fn evaluation_http_error(error: &PurviewGraphError, activity: Activity) -> PurviewDlpHttpError {
    match error {
        PurviewGraphError::Status {
            status,
            retry_after,
        } => upstream_http_error(*status, retry_after.as_deref(), activity, error),
        PurviewGraphError::TokenStatus { status } => {
            upstream_http_error(*status, None, activity, error)
        }
        PurviewGraphError::UserId { .. }
        | PurviewGraphError::Header { .. }
        | PurviewGraphError::Request { .. }
        | PurviewGraphError::Body { .. }
        | PurviewGraphError::ResponseJson { .. }
        | PurviewGraphError::TokenExpiry => PurviewDlpHttpError {
            status: POLICY_BLOCK_STATUS,
            detail: failure_detail(activity, None, &error.to_string()),
            retry_after: None,
        },
    }
}

#[must_use]
pub fn policy_block(activity: Activity) -> PurviewDlpHttpError {
    let mut error = message_error(BLOCKED_ERROR);
    error.detail.insert(
        "activity".to_owned(),
        GuardrailValue::Text(activity_name(activity).to_owned()),
    );
    error
}

#[must_use]
pub fn message_error(message: &str) -> PurviewDlpHttpError {
    PurviewDlpHttpError {
        status: POLICY_BLOCK_STATUS,
        detail: detail(&[("error", GuardrailValue::Text(message.to_owned()))]),
        retry_after: None,
    }
}

mod tests;
