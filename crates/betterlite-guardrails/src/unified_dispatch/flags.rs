use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_translation::StreamingScanKey;

// later source wins, and a present key with a null value still wins, like dict.get(name, previous)
#[must_use]
pub fn resolve_streaming_flag(
    attribute: Option<&GuardrailValue>,
    guardrail_config: Option<&BTreeMap<String, GuardrailValue>>,
    optional_params: &BTreeMap<String, GuardrailValue>,
    name: &str,
    default: &GuardrailValue,
) -> GuardrailValue {
    let sources = [
        attribute,
        guardrail_config.and_then(|config| config.get(name)),
        optional_params.get(name),
    ];
    let mut resolved = default.clone();
    for source in sources.into_iter().flatten() {
        resolved = source.clone();
    }
    resolved
}

#[must_use]
pub fn is_redundant_scan(
    scan_key: Option<&StreamingScanKey>,
    last_scan_key: Option<&StreamingScanKey>,
) -> bool {
    let Some(given) = scan_key else {
        return false;
    };
    last_scan_key == Some(given) || given.has_nothing_to_scan()
}
