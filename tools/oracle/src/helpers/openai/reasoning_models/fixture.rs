use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::reasoning_models::{
    ProviderResolution, ReasoningCapabilities, ResolvedModel, catalogue_declares_default,
};
use serde_json::Value;

use super::Probe;

pub(crate) struct Fixture {
    pub(crate) catalog: CatalogCapabilities,
    entries: BTreeMap<String, Value>,
}

impl Fixture {
    pub(crate) fn new(probe: &Probe) -> Self {
        Self {
            catalog: CatalogCapabilities {
                chat_models: probe.chat_models.clone(),
                text_models: probe.text_models.clone(),
            },
            entries: probe.model_cost.clone(),
        }
    }

    fn entry(&self, request: &CapabilityRequest<'_>) -> Option<&Value> {
        match request.custom_llm_provider {
            None => {
                let known = self.catalog.chat_models.contains(request.model)
                    || self.catalog.text_models.contains(request.model);
                known.then(|| self.entries.get(request.model)).flatten()
            }
            Some(_) => self.entries.get(request.model),
        }
    }
}

impl ReasoningCapabilities for Fixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let entry = self.entry(request);
        ready(
            entry
                .and_then(|fields| fields.get(key))
                .and_then(Value::as_bool)
                == Some(true),
        )
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        let entry = self.entry(request).and_then(|fields| fields.get(key));
        ready(entry.and_then(Value::as_str).map(str::to_owned))
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let entry = self.entry(request);
        ready(
            entry
                .and_then(|fields| fields.get(key))
                .and_then(Value::as_bool)
                == Some(false),
        )
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        let entry = self.entry(request);
        ready(
            entry
                .and_then(|fields| fields.get("supports_response_schema"))
                .and_then(Value::as_bool)
                == Some(true),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self.entries.values().map(Value::to_string).collect();
        catalogue_declares_default(entries.iter().map(String::as_str))
    }
}

pub(crate) struct Resolver(pub(crate) Option<ResolvedModel>);

impl ProviderResolution for Resolver {
    fn resolve_provider(&mut self, _model: &str) -> Option<ResolvedModel> {
        self.0.clone()
    }
}
