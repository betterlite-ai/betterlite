pub mod block_only;
pub mod end_scan;
pub mod error;
pub mod run;
pub mod settings;
pub mod stream_end;

mod tests;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::catalog::RouteCatalog;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::unified_dispatch::registry::TranslationRegistry;

// The guardrail the translation runs, next to the CustomGuardrail settings its gate, name and mask flag come from
pub struct HookGuardrail<'run, G> {
    pub guardrail: &'run G,
    pub base: &'run GuardrailBase,
}

// LiteLLM's three flag sources, later wins: the guardrail's streaming_* attributes, its guardrail_config, the callback's optional_params
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamFlagSources {
    pub attributes: BTreeMap<String, GuardrailValue>,
    pub guardrail_config: BTreeMap<String, GuardrailValue>,
    pub optional_params: BTreeMap<String, GuardrailValue>,
}

pub struct StreamHookParams<'run, T, G> {
    pub guardrail: Option<HookGuardrail<'run, G>>,
    pub registry: &'run TranslationRegistry<T>,
    pub routes: &'run RouteCatalog,
    pub request_data: &'run mut BTreeMap<String, GuardrailValue>,
    pub user_api_key_dict: &'run BTreeMap<String, GuardrailValue>,
    pub request_route: Option<&'run str>,
    pub flags: StreamFlagSources,
    pub buffer_until_moderated_default: bool,
}
