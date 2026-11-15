use std::future::{Future, ready};

use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::{Catalog, get_model_cost_key};
use betterlite_openai::fireworks_ai_chat_params::FireworksCapabilities;

use super::input::CatalogModel;

// The local model map: capability_exact reads these entries, and each predicate answers from the same map.
pub(crate) struct Fixture {
    pub(crate) entries: Catalog,
}

impl Fixture {
    pub(crate) fn load(catalog: &[CatalogModel]) -> Self {
        let mut entries = Catalog::new();
        for model in catalog {
            if let Ok((fields, _warnings)) = ModelFields::parse(&model.model_info) {
                entries.insert(model.key.clone(), fields);
            }
        }
        Self { entries }
    }

    // LiteLLM's _supports_factory reads the provider-prefixed key first, then the bare key, then the cost key.
    fn flag(&self, model: &str, read: fn(&ModelFields) -> Option<bool>) -> Option<bool> {
        let bare = model.strip_prefix("fireworks_ai/").unwrap_or(model);
        let prefixed = if model.starts_with("fireworks_ai/") {
            model.to_owned()
        } else {
            format!("fireworks_ai/{model}")
        };
        let candidates = [prefixed, bare.to_owned(), get_model_cost_key(bare)];
        candidates
            .iter()
            .find_map(|key| self.entries.get(key).and_then(read))
    }
}

impl FireworksCapabilities for Fixture {
    fn supports_function_calling(&mut self, model: &str) -> impl Future<Output = bool> + Send {
        ready(
            self.flag(model, |fields| {
                fields.capabilities.tooling.supports_function_calling
            })
            .unwrap_or(true),
        )
    }

    fn supports_tool_choice(&mut self, model: &str) -> impl Future<Output = bool> + Send {
        ready(
            self.flag(model, |fields| {
                fields.capabilities.tooling.supports_tool_choice
            }) == Some(true),
        )
    }

    fn supports_reasoning(&mut self, model: &str) -> impl Future<Output = bool> + Send {
        ready(
            self.flag(model, |fields| {
                fields.capabilities.reasoning.supports_reasoning
            }) == Some(true),
        )
    }
}
