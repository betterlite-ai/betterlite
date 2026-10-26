use super::{GuardrailExtraValue, GuardrailMessage, GuardrailResponseAction, GuardrailToolParam};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(super) struct RawGenericGuardrailAPIResponse {
    #[serde(default)]
    pub action: GuardrailResponseAction,
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub structured_messages: Option<Vec<GuardrailMessage>>,
    pub blocked_reason: Option<String>,
    pub stream_holdback_chars: Option<Vec<GuardrailExtraValue>>,
}

impl From<RawGenericGuardrailAPIResponse> for super::GenericGuardrailAPIResponse {
    fn from(raw: RawGenericGuardrailAPIResponse) -> Self {
        Self {
            action: raw.action,
            texts: raw.texts,
            images: raw.images,
            tools: raw.tools,
            structured_messages: raw.structured_messages,
            blocked_reason: raw.blocked_reason,
            stream_holdback_chars: raw
                .stream_holdback_chars
                .map(|values| values.iter().map(coerce_stream_holdback_value).collect()),
        }
    }
}

#[must_use]
pub fn coerce_stream_holdback_value(input: &GuardrailExtraValue) -> u32 {
    let parsed = match input {
        GuardrailExtraValue::SignedInteger(number) => u64::try_from(*number).ok(),
        GuardrailExtraValue::UnsignedInteger(number) => Some(*number),
        GuardrailExtraValue::Decimal(number) => decimal_to_u64(*number),
        GuardrailExtraValue::Text(text) => text
            .parse::<u64>()
            .map_or_else(|_| text.parse::<f64>().map_or(None, decimal_to_u64), Some),
        GuardrailExtraValue::Boolean(boolean) => Some(u64::from(*boolean)),
        GuardrailExtraValue::Null
        | GuardrailExtraValue::Array(_)
        | GuardrailExtraValue::Object(_) => None,
    };
    parsed.map_or(0, |number| u32::try_from(number).unwrap_or(0))
}

fn decimal_to_u64(value: f64) -> Option<u64> {
    if value.is_finite() && value >= 0.0_f64 {
        let truncated = value.trunc();
        return format!("{truncated:.0}").parse::<u64>().ok();
    }
    None
}
