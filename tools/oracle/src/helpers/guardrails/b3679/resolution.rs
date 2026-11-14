use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_guardrails::unified_dispatch::registry::{
    TranslationEntry, TranslationRegistry, resolve_endpoint_translation,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryInput {
    pub(super) call_type: CallType,
    pub(super) translation: String,
    #[serde(default)]
    pub(super) chat_completions: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolutionRequest {
    pub(super) request_route: Option<String>,
    pub(super) first_item: Option<GuardrailValue>,
    pub(super) entries: Vec<EntryInput>,
}

pub(super) fn resolve(request: &ResolutionRequest) -> HelperResult {
    let catalog = RouteCatalog::parse(CATALOG).or_config_error()?;
    let registry = TranslationRegistry {
        entries: request
            .entries
            .iter()
            .map(|entry| TranslationEntry {
                call_type: entry.call_type,
                translation: entry.translation.clone(),
                chat_completions: entry.chat_completions,
            })
            .collect(),
    };
    let resolved = resolve_endpoint_translation(
        &registry,
        &catalog,
        request.request_route.as_deref(),
        request.first_item.as_ref(),
    );
    match resolved {
        Some((call_type, translation)) => Ok(json!({
            "call_type": call_type,
            "translation": translation,
        })),
        None => Ok(Value::Null),
    }
}
