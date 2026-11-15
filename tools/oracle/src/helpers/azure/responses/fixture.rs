use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::reasoning_models::{ReasoningCapabilities, catalogue_declares_default};
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use serde_json::Value;

use super::input::Install;

pub(crate) struct MapFixture {
    pub(crate) catalog: CatalogCapabilities,
    entries: BTreeMap<String, Value>,
}

impl MapFixture {
    pub(crate) fn new(install: &Install) -> Self {
        Self {
            catalog: CatalogCapabilities {
                chat_models: install.chat_models.iter().cloned().collect::<BTreeSet<_>>(),
                text_models: install.text_models.iter().cloned().collect::<BTreeSet<_>>(),
            },
            entries: install.model_cost.clone(),
        }
    }

    fn field(&self, request: &CapabilityRequest<'_>, key: &str) -> Option<&Value> {
        let entry = match request.custom_llm_provider {
            None => {
                let known = self.catalog.chat_models.contains(request.model)
                    || self.catalog.text_models.contains(request.model);
                known.then(|| self.entries.get(request.model)).flatten()
            }
            Some(_) => self.entries.get(request.model),
        };
        entry?.get(key)
    }
}

impl ReasoningCapabilities for MapFixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(Value::as_bool) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(
            self.field(request, key)
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(Value::as_bool) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(
            self.field(request, "supports_response_schema")
                .and_then(Value::as_bool)
                == Some(true),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self.entries.values().map(Value::to_string).collect();
        catalogue_declares_default(entries.iter().map(String::as_str))
    }
}

impl ResponsesCapabilities for MapFixture {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        let key = model.to_owned();
        let answer = self.entries.get(model).map(|entry| ModelInfo::Found {
            key,
            supports_reasoning: entry.get("supports_reasoning").and_then(Value::as_bool),
        });
        ready(answer.unwrap_or(ModelInfo::Unavailable))
    }
}
