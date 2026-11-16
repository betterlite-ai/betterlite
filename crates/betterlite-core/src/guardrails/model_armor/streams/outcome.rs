use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::assembly::Assembly;
use super::wire::StreamWire;

// LiteLLM's _resolve_streaming_outcome: whether to refuse the buffered stream, and the rewrite to emit when it is not refused.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub blocked: bool,
    pub sanitized: Option<String>,
}

#[must_use]
pub fn resolve_outcome<W: StreamWire>(
    wire: &W,
    mask: bool,
    content: &str,
    assembled: &Assembly,
    armor: &GuardrailValue,
) -> Outcome {
    if wire.should_block(armor, mask) {
        return Outcome {
            blocked: true,
            sanitized: None,
        };
    }
    if !mask {
        return Outcome::default();
    }
    let sanitized = wire
        .sanitized_content(armor)
        .filter(|text| !text.is_empty());
    let Some(sanitized) = sanitized else {
        // An outstanding redaction with nothing to apply would hand the matched content back.
        return Outcome {
            blocked: wire.has_deidentify(armor),
            sanitized: None,
        };
    };
    if sanitized.as_str() == content {
        return Outcome::default();
    }
    if !matches!(assembled, Assembly::Model(_)) {
        tracing::warn!(
            "Model Armor: sanitized content cannot be re-emitted on this streaming endpoint, blocking the response instead"
        );
        return Outcome {
            blocked: true,
            sanitized: None,
        };
    }
    Outcome {
        blocked: false,
        sanitized: Some(sanitized),
    }
}
