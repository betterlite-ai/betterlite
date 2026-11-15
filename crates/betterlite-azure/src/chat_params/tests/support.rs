use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object as ParamsObject};
use betterlite_openai::reasoning_models::gpt_5::Mapping as Gpt5Mapping;
use betterlite_openai::reasoning_models::{
    DropParams, ReasoningCapabilities, catalogue_declares_default, effort_resolves_to_none,
};
use betterlite_openai::response_events::NativeStreamingSupport;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use crate::chat_params::capabilities::AzureCapabilities;
use crate::chat_params::error::ChatParamsError;
use crate::chat_params::gpt5;
use crate::chat_params::o_series;

pub(super) const MODEL_COST: &str = r#"{
  "azure/gpt-5.1": {
    "supports_tool_choice": true,
    "supports_function_calling": true,
    "supports_parallel_function_calling": true,
    "supports_response_schema": true,
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true,
    "default_reasoning_effort": "none"
  },
  "gpt-5.1": {
    "supports_tool_choice": true,
    "supports_function_calling": true,
    "supports_parallel_function_calling": true,
    "supports_response_schema": true,
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true,
    "default_reasoning_effort": "none"
  },
  "azure/gpt-5.2": {
    "supports_tool_choice": true,
    "supports_function_calling": true,
    "supports_parallel_function_calling": true,
    "supports_response_schema": true,
    "supports_reasoning": true,
    "supports_none_reasoning_effort": true,
    "default_reasoning_effort": "medium"
  },
  "azure/gpt-5.3": {
    "supports_tool_choice": true,
    "supports_function_calling": true,
    "supports_parallel_function_calling": true,
    "supports_response_schema": true,
    "supports_reasoning": true,
    "supports_low_reasoning_effort": false,
    "supports_minimal_reasoning_effort": false
  },
  "azure/o1-preview": {"supports_native_streaming": true, "supports_reasoning": true},
  "azure/o1-mini": {"supports_native_streaming": false, "supports_reasoning": true},
  "o1-mini": {"supports_reasoning": true, "supports_system_messages": false},
  "gpt-4o": {"supports_tool_choice": true, "supports_function_calling": true}
}"#;

pub(super) struct CapabilityFixture {
    pub(super) catalog: CatalogCapabilities,
    entries: BTreeMap<String, Value>,
}

impl CapabilityFixture {
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

impl ReasoningCapabilities for CapabilityFixture {
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

pub(super) struct StreamFixture {
    entries: BTreeMap<String, Value>,
}

impl NativeStreamingSupport for StreamFixture {
    type Error = std::io::Error;

    fn supports_native_streaming(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Result<Json, std::io::Error>> + Send {
        let key = request.custom_llm_provider.map_or_else(
            || request.model.to_owned(),
            |provider| format!("{provider}/{}", request.model),
        );
        let streaming = self.entries.get(&key).map(|entry| {
            entry
                .get("supports_native_streaming")
                .and_then(Value::as_bool)
        });
        ready(streaming.map_or_else(
            || Err(std::io::Error::other("the model map has no such entry")),
            |answer| Ok(answer.map_or(Json::Null, Json::Bool)),
        ))
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn model_cost() -> BTreeMap<String, Value> {
    serde_json::from_str(MODEL_COST).unwrap()
}

fn fixture() -> CapabilityFixture {
    let entries = model_cost();
    CapabilityFixture {
        catalog: CatalogCapabilities {
            chat_models: entries.keys().cloned().collect(),
            text_models: BTreeSet::new(),
        },
        entries,
    }
}

pub(super) fn params(text: &'static str) -> Vec<&'static str> {
    text.split_whitespace().collect()
}

pub(super) fn gpt5_supported(model: &str) -> Vec<&'static str> {
    let mut capabilities = fixture();
    let catalog = capabilities.catalog.clone();
    runtime().block_on(gpt5::supported_params(&mut capabilities, model, &catalog))
}

pub(super) fn o_series_supported(model: &str) -> Vec<&'static str> {
    let mut capabilities = fixture();
    let catalog = capabilities.catalog.clone();
    runtime().block_on(o_series::supported_params(
        &mut capabilities,
        model,
        &catalog,
    ))
}

pub(super) fn resolves_none(model: &str) -> bool {
    let mut capabilities = fixture();
    let mut azure = AzureCapabilities {
        inner: &mut capabilities,
    };
    runtime().block_on(effort_resolves_to_none(&mut azure, model, None))
}

pub(super) fn resolves_none_with(map_text: &str, model: &str) -> bool {
    let entries: BTreeMap<String, Value> = serde_json::from_str(map_text).unwrap();
    let chat_models = entries.keys().cloned().collect();
    let mut capabilities = CapabilityFixture {
        catalog: CatalogCapabilities {
            chat_models,
            text_models: BTreeSet::new(),
        },
        entries,
    };
    let mut azure = AzureCapabilities {
        inner: &mut capabilities,
    };
    runtime().block_on(effort_resolves_to_none(&mut azure, model, None))
}

pub(super) fn fake(model: Option<&str>, stream: Option<bool>, provider: Option<&str>) -> bool {
    fake_in(MODEL_COST, model, stream, provider)
}

pub(super) fn fake_in(
    map_text: &str,
    model: Option<&str>,
    stream: Option<bool>,
    provider: Option<&str>,
) -> bool {
    let mut capabilities = StreamFixture {
        entries: serde_json::from_str(map_text).unwrap(),
    };
    runtime().block_on(o_series::should_fake_stream(
        &mut capabilities,
        model,
        stream,
        provider,
    ))
}

pub(super) const TOOLS: &str = r#"[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"a":{"type":"string","pattern":"\\p{L}"}}}}},{"type":"function","function":{"name":"g","parameters":{"type":"object","anyOf":[{"properties":{"a":{"type":"string"}}},{"properties":{"b":{"type":"number"}}}]}}}]"#;

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn messages(text: &str) -> Vec<Message> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn mapped(
    model: &str,
    non_default: &str,
    optional: &str,
    drop: DropParams,
) -> Result<Object, ChatParamsError> {
    let mut capabilities = fixture();
    let catalog = capabilities.catalog.clone();
    let mut requested: ParamsObject = serde_json::from_str(non_default).unwrap();
    let mut existing: ParamsObject = serde_json::from_str(optional).unwrap();
    let mut mapping = Gpt5Mapping {
        model,
        non_default_params: &mut requested,
        optional_params: &mut existing,
        drop_params: drop,
        catalog: &catalog,
    };
    runtime().block_on(gpt5::map_params(&mut capabilities, &mut mapping))?;
    Ok(serde_json::from_value(Value::Object(existing)).unwrap())
}

pub(super) fn composed(
    model: &str,
    params: &str,
    messages_text: &str,
    system_first: bool,
) -> Object {
    let optional = mapped(model, params, "{}", DropParams::default()).unwrap();
    gpt5::transform_request(model, &messages(messages_text), &optional, system_first).unwrap()
}

pub(super) fn o_series_body(
    model: &str,
    optional: &str,
    messages_text: &str,
    system_first: bool,
) -> Object {
    let mut capabilities = fixture();
    runtime()
        .block_on(o_series::transform_request(
            &mut capabilities,
            model,
            &messages(messages_text),
            &object(optional),
            system_first,
        ))
        .unwrap()
}
