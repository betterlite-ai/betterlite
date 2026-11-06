use std::str;

use indexmap::IndexMap;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{RequestData, store_pii_tokens};
use crate::presidio_actions::tokens::number_spans;
use crate::presidio_analysis::wire::AnalyzeResponseItem;

const ERROR_SAMPLE_CHARS: usize = 200;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AnonymizeItem {
    #[serde(default)]
    pub entity_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AnonymizeResponse {
    pub text: String,
    #[serde(default)]
    pub items: Vec<AnonymizeItem>,
}

pub struct AnonymizeTextCall<'call> {
    pub text: &'call str,
    pub analyze_results: &'call [AnalyzeResponseItem],
    pub output_parse_pii: bool,
    pub masked_entity_count: &'call mut IndexMap<String, usize>,
    pub request_data: Option<&'call mut RequestData>,
}

#[derive(Debug, Serialize)]
struct AnonymizePayload<'payload> {
    text: &'payload str,
    analyzer_results: &'payload [AnalyzeResponseItem],
}

pub async fn post_anonymize(
    anonymizer: &PresidioAnonymizer,
    text: &str,
    analyze_results: &[AnalyzeResponseItem],
) -> Result<AnonymizeResponse, PresidioActionsError> {
    let url = format!("{}anonymize", anonymizer.settings.anonymizer_api_base);
    let payload = AnonymizePayload {
        text,
        analyzer_results: analyze_results,
    };
    let response = anonymizer
        .client
        .post(url)
        .header(ACCEPT, "application/json")
        .timeout(anonymizer.settings.timeout)
        .json(&payload)
        .send()
        .await
        .map_err(|source| PresidioActionsError::AnonymizerRequest { source })?;
    let status = response.status();
    if status.as_u16() >= 400 {
        let body = read_body(response, ERROR_SAMPLE_CHARS.saturating_mul(4)).await?;
        return Err(PresidioActionsError::AnonymizerHttpStatus {
            status: status.as_u16(),
            body: prefix_text(&body, ERROR_SAMPLE_CHARS),
        });
    }
    let content_type = response_mimetype(&response);
    if !content_type.contains("application/json") {
        let body = read_body(response, ERROR_SAMPLE_CHARS.saturating_mul(4)).await?;
        return Err(PresidioActionsError::AnonymizerContentType {
            content_type,
            body: prefix_text(&body, ERROR_SAMPLE_CHARS),
        });
    }
    let body = read_body(response, MAX_RESPONSE_BYTES).await?;
    serde_json::from_slice(&body)
        .map_err(|source| PresidioActionsError::AnonymizerResponseJson { source })
}

pub async fn anonymize_text(
    anonymizer: &PresidioAnonymizer,
    call: AnonymizeTextCall<'_>,
) -> Result<String, PresidioActionsError> {
    let AnonymizeTextCall {
        text,
        analyze_results,
        output_parse_pii,
        masked_entity_count,
        request_data,
    } = call;
    if analyze_results.is_empty() {
        return Ok(text.to_owned());
    }
    let redacted = post_anonymize(anonymizer, text, analyze_results).await?;
    if !output_parse_pii {
        return Ok(finalize_simple(&redacted, masked_entity_count));
    }
    finalize_numbered_tokens(text, analyze_results, request_data, masked_entity_count)
}

#[must_use]
pub fn finalize_simple(
    redacted: &AnonymizeResponse,
    masked_entity_count: &mut IndexMap<String, usize>,
) -> String {
    for item in &redacted.items {
        if let Some(entity_type) = &item.entity_type {
            let count = masked_entity_count.entry(entity_type.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    redacted.text.clone()
}

pub fn finalize_numbered_tokens(
    text: &str,
    analyze_results: &[AnalyzeResponseItem],
    request_data: Option<&mut RequestData>,
    masked_entity_count: &mut IndexMap<String, usize>,
) -> Result<String, PresidioActionsError> {
    let numbered = number_spans(text, analyze_results, masked_entity_count)?;
    let stored = request_data.map_or_else(
        || {
            tracing::warn!(
                "Presidio anonymize_text ran without request_data; PII tokens are not stored per request"
            );
            Ok(())
        },
        |target| store_pii_tokens(target, &numbered.tokens),
    );
    stored?;
    Ok(numbered.text)
}

fn response_mimetype(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map_or_else(
            || "application/octet-stream".to_owned(),
            |mimetype| mimetype.trim().to_owned(),
        )
}

async fn read_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PresidioActionsError> {
    let limit_bytes = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes)
    {
        return Err(PresidioActionsError::ResponseTooLarge { limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| PresidioActionsError::AnonymizerRequest { source })?
    {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(PresidioActionsError::ResponseTooLarge { limit });
        }
    }
    Ok(body)
}

fn prefix_text(bytes: &[u8], max_chars: usize) -> String {
    let valid_len = match str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) => error.valid_up_to(),
    };
    let Some(valid) = bytes.get(..valid_len) else {
        return String::new();
    };
    let Ok(text) = str::from_utf8(valid) else {
        return String::new();
    };
    text.chars().take(max_chars).collect()
}
