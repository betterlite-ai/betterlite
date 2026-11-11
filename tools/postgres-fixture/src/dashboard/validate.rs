use super::{DashboardFixtureError, SpendLogSeed};

const TEXT_BYTES: usize = 4096;
const JSON_BYTES: usize = 0x1_0000;

pub(super) fn validate_log(
    row: usize,
    log: &SpendLogSeed,
    keys: usize,
) -> Result<(), DashboardFixtureError> {
    if log.key_index >= keys {
        return Err(DashboardFixtureError::UnknownKey {
            row,
            key_index: log.key_index,
            keys,
        });
    }
    validate_text(row, log)?;
    validate_counters(row, log)?;
    validate_payloads(row, log)
}

fn validate_text(row: usize, log: &SpendLogSeed) -> Result<(), DashboardFixtureError> {
    check_text(row, "request_id", &log.request_id)?;
    check_text(row, "call_type", &log.call_type)?;
    check_text(row, "start_time", &log.start_time)?;
    check_text(row, "end_time", &log.end_time)?;
    for (field, value) in [
        ("completion_start_time", &log.completion_start_time),
        ("model", &log.model),
        ("model_id", &log.model_id),
        ("model_group", &log.model_group),
        ("custom_llm_provider", &log.custom_llm_provider),
        ("user", &log.user),
        ("team_id", &log.team_id),
        ("end_user", &log.end_user),
        ("session_id", &log.session_id),
        ("status", &log.status),
        ("cache_hit", &log.cache_hit),
        ("litellm_call_id", &log.litellm_call_id),
    ] {
        if let Some(text) = value {
            check_text(row, field, text)?;
        }
    }
    Ok(())
}

const fn validate_counters(row: usize, log: &SpendLogSeed) -> Result<(), DashboardFixtureError> {
    if log.total_tokens.is_negative() {
        return Err(DashboardFixtureError::NegativeCounter {
            row,
            field: "total_tokens",
            value: log.total_tokens,
        });
    }
    if log.prompt_tokens.is_negative() {
        return Err(DashboardFixtureError::NegativeCounter {
            row,
            field: "prompt_tokens",
            value: log.prompt_tokens,
        });
    }
    if log.completion_tokens.is_negative() {
        return Err(DashboardFixtureError::NegativeCounter {
            row,
            field: "completion_tokens",
            value: log.completion_tokens,
        });
    }
    if let Some(duration) = log.request_duration_ms
        && duration.is_negative()
    {
        return Err(DashboardFixtureError::NegativeDuration { row });
    }
    Ok(())
}

fn validate_payloads(row: usize, log: &SpendLogSeed) -> Result<(), DashboardFixtureError> {
    for (field, value) in [
        ("metadata", &log.metadata),
        ("request_tags", &log.request_tags),
        ("messages", &log.messages),
        ("response", &log.response),
        ("proxy_server_request", &log.proxy_server_request),
    ] {
        let Some(payload) = value else {
            continue;
        };
        let bytes = serde_json::to_string(payload)
            .map_err(|source| DashboardFixtureError::JsonUnserializable { row, field, source })?;
        if bytes.len() > JSON_BYTES {
            return Err(DashboardFixtureError::JsonTooLong {
                row,
                field,
                bytes: bytes.len(),
                limit: JSON_BYTES,
            });
        }
    }
    Ok(())
}

const fn check_text(
    row: usize,
    field: &'static str,
    text: &str,
) -> Result<(), DashboardFixtureError> {
    if text.len() > TEXT_BYTES {
        return Err(DashboardFixtureError::TextTooLong {
            row,
            field,
            bytes: text.len(),
            limit: TEXT_BYTES,
        });
    }
    Ok(())
}
