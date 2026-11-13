use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::Deserialize;

use super::error::PillarError;
use super::response_headers::build_pillar_response_headers;
use super::settings::{OnFlaggedAction, PillarSettings};
use crate::guardrail_base::failure::GuardrailFailure;

const BLOCKED_STATUS: u16 = 400;
const BLOCKED_ERROR: &str = "Blocked by Pillar Security Guardrail";
const DETECTION_MESSAGE: &str = "Security threats detected";

// LiteLLM's _PillarProtectResponse TypedDict; keys it does not know stay so an unknown-only body still counts
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ProtectResponse {
    #[serde(default)]
    pub flagged: Option<bool>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub scanners: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    pub evidence: Option<Vec<GuardrailValue>>,
    #[serde(default)]
    pub masked_session_messages: Option<Vec<GuardrailValue>>,
    #[serde(flatten)]
    pub other: BTreeMap<String, GuardrailValue>,
}

// LiteLLM's get_metadata_variable_name_from_kwargs
fn metadata_key(data: &BTreeMap<String, GuardrailValue>) -> &'static str {
    if data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    }
}

fn store_bucket(
    data: &mut BTreeMap<String, GuardrailValue>,
    key: &str,
    bucket: BTreeMap<String, GuardrailValue>,
    alias_metadata: bool,
) {
    let value = GuardrailValue::Object(bucket);
    if alias_metadata {
        data.insert("metadata".to_owned(), value.clone());
    }
    data.insert(key.to_owned(), value);
}

fn scanners_value(response: &ProtectResponse) -> GuardrailValue {
    GuardrailValue::Object(response.scanners.clone().unwrap_or_default())
}

fn evidence_value(response: &ProtectResponse) -> GuardrailValue {
    GuardrailValue::Array(response.evidence.clone().unwrap_or_default())
}

fn detection_detail(
    response: &ProtectResponse,
    settings: &PillarSettings,
) -> IndexMap<String, GuardrailValue> {
    let mut pillar_response = BTreeMap::new();
    let session = response
        .session_id
        .as_ref()
        .map_or(GuardrailValue::Null, |id| GuardrailValue::Text(id.clone()));
    pillar_response.insert("session_id".to_owned(), session);
    if settings.include_scanners {
        pillar_response.insert("scanners".to_owned(), scanners_value(response));
    }
    if settings.include_evidence {
        pillar_response.insert("evidence".to_owned(), evidence_value(response));
    }
    IndexMap::from([
        (
            "error".to_owned(),
            GuardrailValue::Text(BLOCKED_ERROR.to_owned()),
        ),
        (
            "detection_message".to_owned(),
            GuardrailValue::Text(DETECTION_MESSAGE.to_owned()),
        ),
        (
            "pillar_response".to_owned(),
            GuardrailValue::Object(pillar_response),
        ),
    ])
}

pub fn process_pillar_response(
    response: &ProtectResponse,
    data: &mut BTreeMap<String, GuardrailValue>,
    settings: &PillarSettings,
) -> Result<(), PillarError> {
    // LiteLLM returns early on an empty body, before writing any metadata
    if *response == ProtectResponse::default() {
        return Ok(());
    }
    let flagged = response.flagged.unwrap_or(false);
    let key = metadata_key(data);
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        None | Some(_) => BTreeMap::new(),
    };
    let alias_metadata =
        key != "metadata" && !matches!(data.get("metadata"), Some(GuardrailValue::Object(_)));
    if let Some(session_id) = response
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        if !bucket.contains_key("pillar_session_id") {
            let stored = GuardrailValue::Text(session_id.to_owned());
            bucket.insert("pillar_session_id".to_owned(), stored);
        }
        let stored = GuardrailValue::Text(session_id.to_owned());
        bucket.insert("pillar_session_id_response".to_owned(), stored);
    }
    bucket.insert(
        "pillar_flagged".to_owned(),
        GuardrailValue::Boolean(flagged),
    );
    if settings.include_scanners {
        bucket.insert("pillar_scanners".to_owned(), scanners_value(response));
    }
    if settings.include_evidence {
        bucket.insert("pillar_evidence".to_owned(), evidence_value(response));
    }
    if flagged {
        tracing::warn!("Pillar Guardrail: Threat detected");
        match settings.on_flagged_action {
            OnFlaggedAction::Block => {
                // the writes above stay on the request data, as they do before LiteLLM raises
                store_bucket(data, key, bucket, alias_metadata);
                let detail = detection_detail(response, settings);
                return Err(PillarError::Http(GuardrailFailure::HttpObject {
                    status: BLOCKED_STATUS,
                    detail,
                }));
            }
            OnFlaggedAction::Mask => {
                tracing::info!("Pillar Guardrail: Masking mode - masking flagged content");
                let masked = response.masked_session_messages.clone().unwrap_or_default();
                if masked.is_empty() {
                    tracing::warn!(
                        "Pillar Guardrail: Masking requested but no masked_session_messages in response"
                    );
                } else {
                    data.insert("messages".to_owned(), GuardrailValue::Array(masked));
                }
            }
            OnFlaggedAction::Monitor => {
                tracing::info!(
                    "Pillar Guardrail: Monitoring mode - allowing flagged content to proceed"
                );
            }
        }
    }
    build_pillar_response_headers(&mut bucket);
    store_bucket(data, key, bucket, alias_metadata);
    Ok(())
}
