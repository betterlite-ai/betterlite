use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_redact::structured::SensitiveDataMasker;

use super::models::OverrideKind;

// config_override_endpoints.py's _mask_sensitive_fields: only a listed field's string value is masked.
#[must_use]
pub fn mask_sensitive_fields(kind: OverrideKind, config: &JsonMap) -> JsonMap {
    let sensitive = kind
        .fields()
        .iter()
        .filter(|field| field.sensitive)
        .map(|field| field.name)
        .collect::<BTreeSet<&str>>();
    let masker = SensitiveDataMasker::default();
    config
        .iter()
        .map(|(name, value)| {
            let output = match value {
                JsonValue::String(text) if sensitive.contains(name.as_str()) => {
                    JsonValue::String(masker.mask_value(text))
                }
                JsonValue::String(_)
                | JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => value.clone(),
            };
            (name.clone(), output)
        })
        .collect()
}
