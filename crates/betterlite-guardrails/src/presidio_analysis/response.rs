use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::value::RawValue;

use super::body::read_text_prefix;
use super::error::PresidioAnalysisError;
use super::wire::{AnalyzeBody, AnalyzeRequest, AnalyzeResponseItem, ListEntry};
use super::{AnalyzerSettings, ERROR_SAMPLE_CHARS, PresidioAnalyzer, json_body};

fn json_type_name(raw: &RawValue) -> &'static str {
    match raw.get().as_bytes().first() {
        Some(b'"') => "string",
        Some(b't' | b'f') => "boolean",
        Some(b'n') => "null",
        Some(_) if raw.get().contains(['.', 'e', 'E']) => "float",
        _ => "integer",
    }
}

// Fail closed when PII protection is configured: a skipped analysis there would let PII through unmasked.
fn fail_on_invalid_response(
    settings: &AnalyzerSettings,
    reason: &str,
) -> Result<Vec<AnalyzeResponseItem>, PresidioAnalysisError> {
    if !settings.pii_entities.is_empty() || settings.output_parse_pii || settings.apply_to_output {
        return Err(PresidioAnalysisError::GuardrailRaised {
            guardrail_name: settings.guardrail_name.clone(),
            message: format!(
                "Presidio analyzer returned invalid response; cannot verify PII when PII protection is configured: {reason}"
            ),
        });
    }
    tracing::warn!(
        reason,
        "Presidio analyzer response invalid, returning an empty list"
    );
    Ok(Vec::new())
}

// aiohttp reports the bare mimetype, application/octet-stream when the header is missing
fn response_mimetype(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map_or_else(
            || "application/octet-stream".to_owned(),
            |mimetype| mimetype.trim().to_ascii_lowercase(),
        )
}

pub(super) async fn post_analyze(
    analyzer: &PresidioAnalyzer,
    payload: &AnalyzeRequest,
) -> Result<Vec<AnalyzeResponseItem>, PresidioAnalysisError> {
    let settings = &analyzer.settings;
    let analyze_url = format!("{}analyze", settings.analyzer_api_base);
    let response = analyzer
        .client
        .post(analyze_url)
        .header(ACCEPT, "application/json")
        .timeout(settings.timeout)
        .json(payload)
        .send()
        .await
        .map_err(|source| PresidioAnalysisError::Request { source })?;
    let status = response.status();
    if status.as_u16() >= 400 {
        let error_body = read_text_prefix(response, ERROR_SAMPLE_CHARS).await?;
        return fail_on_invalid_response(
            settings,
            &format!(
                "HTTP {} from Presidio analyzer: {error_body}",
                status.as_u16()
            ),
        );
    }
    let content_type = response_mimetype(&response);
    if !content_type.contains("application/json") {
        let error_body = read_text_prefix(response, ERROR_SAMPLE_CHARS).await?;
        return fail_on_invalid_response(
            settings,
            &format!(
                "expected application/json Content-Type but received '{content_type}'; body: '{error_body}'"
            ),
        );
    }
    interpret_body(settings, json_body(response).await?)
}

fn interpret_body(
    settings: &AnalyzerSettings,
    body: AnalyzeBody,
) -> Result<Vec<AnalyzeResponseItem>, PresidioAnalysisError> {
    match body {
        AnalyzeBody::List(entries) => {
            let mut items = Vec::with_capacity(entries.len());
            for entry in entries {
                match entry {
                    ListEntry::Item(item) => items.push(item),
                    ListEntry::Invalid(value) => tracing::warn!(
                        sample = %value.get().chars().take(100).collect::<String>(),
                        "Skipping invalid Presidio result item"
                    ),
                }
            }
            Ok(items)
        }
        AnalyzeBody::ErrorObject { error } => {
            fail_on_invalid_response(settings, &format!("error: {}", error.get()))
        }
        AnalyzeBody::Item(item) => Ok(vec![item]),
        AnalyzeBody::UnparsedDict => {
            fail_on_invalid_response(settings, "failed to parse dict response")
        }
        AnalyzeBody::Unexpected(value) => fail_on_invalid_response(
            settings,
            &format!(
                "unexpected type {} (expected list or dict), response: {}",
                json_type_name(&value),
                value
                    .get()
                    .chars()
                    .take(ERROR_SAMPLE_CHARS)
                    .collect::<String>()
            ),
        ),
    }
}
