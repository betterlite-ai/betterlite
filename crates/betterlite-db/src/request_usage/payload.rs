use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::autorouter_facts::turn::TurnPayload;
use crate::daily_records::projection::DailyPayload;

// SpendLogsPayload as _enqueue_tool_usage_transaction reads it; a field the source indexes stays optional so its absence fails where the source raises.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ToolUsagePayload {
    pub request_id: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
    pub total_tokens: Option<i64>,
}

// The kwargs _enqueue_tool_registry_upsert reads, with every nested value kept as the shape the caller passed.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DiscoveryKwargs {
    pub standard_logging_object: Option<Json>,
    pub tools: Option<Json>,
    pub passthrough_logging_payload: Option<Json>,
    pub litellm_params: Option<Json>,
}

// The spend log payload the turn enqueue and its baseline capture read, one decoded view per source read.
pub struct AutorouterPayload {
    pub turn: TurnPayload,
    pub daily: DailyPayload,
    pub metadata: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PayloadEnvelope {
    metadata: Option<String>,
}

// payload.get("metadata") stays whatever the source's own read makes of it; the enqueue decides whether it names a turn.
pub fn decode_autorouter_payload(bytes: &[u8]) -> Result<AutorouterPayload, serde_json::Error> {
    let turn = serde_json::from_slice(bytes)?;
    let daily = serde_json::from_slice(bytes)?;
    let envelope: PayloadEnvelope = serde_json::from_slice(bytes)?;
    Ok(AutorouterPayload {
        turn,
        daily,
        metadata: envelope.metadata,
    })
}
