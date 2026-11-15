use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC};

// urllib.parse.quote(value, safe="") keeps only letters, digits and _.-~
pub(crate) const QUOTE_SAFE_NONE: AsciiSet = NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

// python json.dumps(data, ensure_ascii=False, separators=(",", ":")) is serde_json's compact form
#[must_use]
pub fn json_text(data: &GuardrailValue) -> String {
    serde_json::to_string(data).unwrap_or_else(|error| {
        tracing::error!(%error, "Pillar Guardrail: value is not JSON");
        String::from("null")
    })
}
