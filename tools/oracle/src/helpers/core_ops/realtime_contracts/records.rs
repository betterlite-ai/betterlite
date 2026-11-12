use super::failure;
use crate::helpers::HelperResult;
use betterlite_core::api::realtime_contracts::decode::{
    RealtimeContract, RealtimeContractKind, RealtimeDecodeLimits, decode_realtime_contract,
};
use betterlite_core::api::realtime_contracts::events::{
    AudioTranscriptionUsage, DurationUsage, InputTokenDetails, TokenUsage,
};
use serde::Deserialize;
use serde_json::{Value, json};
const LIMITS: RealtimeDecodeLimits = RealtimeDecodeLimits { max_bytes: 1 << 20 };
#[derive(Debug, Deserialize)]
pub(crate) struct RecordsInput {
    groups: Vec<RecordGroup>,
}
#[derive(Debug, Deserialize)]
struct RecordGroup {
    kind: RecordKind,
    #[serde(default)]
    documents: Vec<Value>,
    #[serde(default)]
    usage: Vec<UsageDocument>,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RecordKind {
    Expiry,
    Session,
    ClientSecretRequest,
    ClientSecretResponse,
    TranscriptionRequest,
    TranscriptionResponse,
    Usage,
}
impl RecordKind {
    const fn contract(self) -> Option<RealtimeContractKind> {
        match self {
            Self::Usage => None,
            Self::Expiry => Some(RealtimeContractKind::Expiry),
            Self::Session => Some(RealtimeContractKind::Session),
            Self::ClientSecretRequest => Some(RealtimeContractKind::ClientSecretRequest),
            Self::ClientSecretResponse => Some(RealtimeContractKind::ClientSecretResponse),
            Self::TranscriptionRequest => Some(RealtimeContractKind::TranscriptionRequest),
            Self::TranscriptionResponse => Some(RealtimeContractKind::TranscriptionResponse),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UsageKind {
    Tokens,
    Duration,
}
#[derive(Debug, Deserialize)]
struct UsageDocument {
    #[serde(rename = "type")]
    usage_type: UsageKind,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    text_tokens: i64,
    audio_tokens: i64,
    seconds: f64,
}

pub(super) fn records(input: &RecordsInput) -> HelperResult {
    let mut rendered = Vec::new();
    let mut resolved = Vec::new();
    for group in &input.groups {
        let Some(kind) = group.kind.contract() else {
            for document in &group.usage {
                rendered.push(usage_value(document)?);
            }
            continue;
        };
        for document in &group.documents {
            let bytes = serde_json::to_vec(document).map_err(|error| failure(error.to_string()))?;
            let record = decode_realtime_contract(&bytes, kind, LIMITS)
                .map_err(|error| failure(error.to_string()))?;
            if let RealtimeContract::TranscriptionRequest(request) = &record {
                resolved.push(request.resolved_model().map(str::to_owned));
            }
            rendered
                .push(serde_json::to_value(&record).map_err(|error| failure(error.to_string()))?);
        }
    }
    Ok(json!({"records": rendered, "resolved_models": resolved}))
}
fn usage_value(document: &UsageDocument) -> Result<Value, Box<crate::helpers::HelperError>> {
    let usage = match document.usage_type {
        UsageKind::Tokens => AudioTranscriptionUsage::Tokens(Box::new(TokenUsage {
            input_tokens: document.input_tokens,
            output_tokens: document.output_tokens,
            total_tokens: document.total_tokens,
            input_token_details: InputTokenDetails {
                text_tokens: document.text_tokens,
                audio_tokens: document.audio_tokens,
            },
        })),
        UsageKind::Duration => AudioTranscriptionUsage::Duration(DurationUsage {
            seconds: document.seconds,
        }),
    };
    serde_json::to_value(usage).map_err(|error| failure(error.to_string()))
}
