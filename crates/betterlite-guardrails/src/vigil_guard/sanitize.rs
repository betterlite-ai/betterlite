use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::analysis::Analysis;

#[must_use]
pub fn resolve_sanitized_text(original: &str, analysis: &Analysis) -> String {
    for value in [
        analysis.sanitized_text.as_ref(),
        analysis.output_text.as_ref(),
    ] {
        if let Some(GuardrailValue::Text(text)) = value {
            return text.clone();
        }
    }
    original.to_owned()
}
