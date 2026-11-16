use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_foundry::chat::capabilities::CostMap;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::reasoning_models::{ReasoningCapabilities, catalogue_declares_default};
use serde_json::Value;

use super::input::Input;

pub(crate) struct Installed {
    pub(crate) capabilities: ChatCapabilities,
    pub(crate) costs: CostKeys,
    pub(crate) catalog: CatalogCapabilities,
    pub(crate) embedding: BTreeSet<String>,
}

pub(crate) struct ChatCapabilities {
    entries: BTreeMap<String, Value>,
    resolvable: BTreeSet<String>,
}

pub(crate) struct CostKeys {
    keys: BTreeSet<String>,
}

impl Installed {
    pub(crate) fn new(input: &Input) -> Self {
        let chat: BTreeSet<String> = input.chat_models.iter().cloned().collect();
        let text: BTreeSet<String> = input.text_models.iter().cloned().collect();
        let embedding: BTreeSet<String> = input.embedding_models.iter().cloned().collect();
        let resolvable = chat.union(&text).cloned().collect();
        Self {
            capabilities: ChatCapabilities {
                entries: input.model_cost.clone(),
                resolvable,
            },
            costs: CostKeys {
                keys: input.model_cost.keys().cloned().collect(),
            },
            catalog: CatalogCapabilities {
                chat_models: chat,
                text_models: text,
            },
            embedding,
        }
    }
}

impl CostMap for CostKeys {
    fn contains(&self, model: &str) -> bool {
        self.keys.contains(model)
    }
}

impl ChatCapabilities {
    // A bare name answers only when the OpenAI catalog lists it, as LiteLLM's lookup does.
    fn field(&self, model: &str, key: &str) -> Option<&Value> {
        let bare = model.split_once('/').map(|(_, name)| name);
        if bare.is_none() && !self.resolvable.contains(model) {
            return None;
        }
        let own = self.entries.get(model).and_then(|entry| entry.get(key));
        own.or_else(|| {
            bare.and_then(|name| self.entries.get(name))
                .and_then(|entry| entry.get(key))
        })
    }

    fn flag(&self, model: &str, key: &str) -> Option<bool> {
        self.field(model, key).and_then(Value::as_bool)
    }
}

impl ReasoningCapabilities for ChatCapabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.flag(request.model, key) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        let answer = self
            .field(request.model, key)
            .and_then(|value| value.as_str().map(str::to_owned));
        ready(answer)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.flag(request.model, key) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(self.flag(request.model, "supports_response_schema") == Some(true))
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self.entries.values().map(Value::to_string).collect();
        catalogue_declares_default(entries.iter().map(String::as_str))
    }
}
