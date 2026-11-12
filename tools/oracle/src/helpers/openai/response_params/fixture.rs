use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::ReasoningCapabilities;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use serde_json::Value;

pub(crate) struct Fixture {
    entries: BTreeMap<String, Value>,
}

impl Fixture {
    pub(crate) fn new(entries: &BTreeMap<String, Value>) -> Self {
        Self {
            entries: entries.clone(),
        }
    }

    fn field(&self, model: &str, key: &str) -> Option<&Value> {
        if let Some(entry) = self.entries.get(model) {
            return entry.get(key);
        }
        let (_, name) = model.rsplit_once('/')?;
        self.entries.get(name)?.get(key)
    }
}

impl ReasoningCapabilities for Fixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request.model, key).and_then(Value::as_bool) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(
            self.field(request.model, key)
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request.model, key).and_then(Value::as_bool) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(
            self.field(request.model, "supports_response_schema")
                .is_some(),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.get("default_reasoning_effort").is_some())
    }
}

impl ResponsesCapabilities for Fixture {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(
            self.entries
                .get(model)
                .map_or(ModelInfo::Unavailable, |entry| ModelInfo::Found {
                    key: model.to_owned(),
                    supports_reasoning: entry.get("supports_reasoning").and_then(Value::as_bool),
                }),
        )
    }
}
