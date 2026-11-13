use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::{CATALOG, RouteCatalog};
use betterlite_guardrails::unified_dispatch::registry::{
    TranslationEntry, TranslationRegistry, resolve_endpoint_translation,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryInput {
    call_type: CallType,
    translation: String,
    #[serde(default)]
    chat_completions: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    EndpointResolution {
        request_route: Option<String>,
        first_item: Option<GuardrailValue>,
        entries: Vec<EntryInput>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::EndpointResolution {
                request_route,
                first_item,
                entries,
            } => resolve(request_route.as_deref(), first_item.as_ref(), entries),
        }
    }
}

fn resolve(
    request_route: Option<&str>,
    first_item: Option<&GuardrailValue>,
    entries: &[EntryInput],
) -> HelperResult {
    let catalog = RouteCatalog::parse(CATALOG).or_config_error()?;
    let registry = TranslationRegistry {
        entries: entries
            .iter()
            .map(|entry| TranslationEntry {
                call_type: entry.call_type,
                translation: entry.translation.clone(),
                chat_completions: entry.chat_completions,
            })
            .collect(),
    };
    let resolved = resolve_endpoint_translation(&registry, &catalog, request_route, first_item);
    match resolved {
        Some((call_type, translation)) => Ok(json!({
            "call_type": call_type,
            "translation": translation,
        })),
        None => Ok(Value::Null),
    }
}
