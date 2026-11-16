use std::collections::BTreeSet;
use std::future::Future;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ModelFamily, Object, model_family,
};
use betterlite_openai::reasoning_models::gpt_5::{self, Mapping as Gpt5Mapping};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities, ReasoningError};
use betterlite_openai::xai_chat::params::supported_with;

use super::{AZURE_AI_PREFIX, base_config};

pub const TOOL_CHOICE: &str = "supports_tool_choice";

// Sync because the forwarded lookups hold the map across an await in a Send future
pub trait CostMap: Sync {
    fn contains(&self, model: &str) -> bool;
}

impl CostMap for ModelCatalog {
    fn contains(&self, model: &str) -> bool {
        self.metadata(model).is_some()
    }
}

// LiteLLM reads azure_ai/<model> whenever the map carries it, and the bare name otherwise
#[must_use]
pub fn model_map_lookup_name(model: &str, cost_map: &dyn CostMap) -> String {
    let prefixed = if model.starts_with(AZURE_AI_PREFIX) {
        model.to_owned()
    } else {
        format!("{AZURE_AI_PREFIX}{model}")
    };
    if cost_map.contains(&prefixed) {
        prefixed
    } else {
        model.to_owned()
    }
}

fn gpt_reasoning_name(model: &str) -> bool {
    let empty = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    model_family(model, &empty) == ModelFamily::GptReasoning
}

// LiteLLM's GPT-5 map lookup exempts supports_tool_choice, which is asked with the routing name
fn lookup_model(model: &str, key: &str, cost_map: &dyn CostMap) -> String {
    if key == TOOL_CHOICE {
        model.to_owned()
    } else {
        model_map_lookup_name(model, cost_map)
    }
}

pub struct FoundryCapabilities<'capabilities, Capabilities> {
    pub inner: &'capabilities mut Capabilities,
    pub cost_map: &'capabilities dyn CostMap,
}

macro_rules! forward_lookup {
    ($method:ident -> $answer:ty) => {
        fn $method(
            &mut self,
            request: &CapabilityRequest<'_>,
            key: &str,
        ) -> impl Future<Output = $answer> + Send {
            async move {
                let model = lookup_model(request.model, key, self.cost_map);
                let normalized = CapabilityRequest {
                    model: model.as_str(),
                    custom_llm_provider: request.custom_llm_provider,
                };
                self.inner.$method(&normalized, key).await
            }
        }
    };
}

impl<Capabilities> ReasoningCapabilities for FoundryCapabilities<'_, Capabilities>
where
    Capabilities: ReasoningCapabilities + Send,
{
    forward_lookup!(supports_factory -> bool);
    forward_lookup!(declared_value_factory -> Option<String>);
    forward_lookup!(is_explicitly_disabled_factory -> bool);

    async fn supports_response_schema(&mut self, request: &CapabilityRequest<'_>) -> bool {
        self.inner.supports_response_schema(request).await
    }

    fn catalogue_declares_default(&mut self) -> bool {
        self.inner.catalogue_declares_default()
    }
}

pub struct RequestParams<'request> {
    pub model: &'request str,
    pub non_default_params: &'request mut Object,
    pub optional_params: &'request mut Object,
    pub drop_params: DropParams,
    pub catalog: &'request CatalogCapabilities,
}

pub async fn supported_params<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    catalog: &CatalogCapabilities,
    cost_map: &dyn CostMap,
) -> Vec<&'static str>
where
    Capabilities: ReasoningCapabilities + Send,
{
    let mut params = if gpt_reasoning_name(model) {
        let mut wrapped = FoundryCapabilities {
            inner: &mut *capabilities,
            cost_map,
        };
        gpt_5::supported_params(&mut wrapped, model, catalog).await
    } else {
        base_config().supported_params(model, catalog)
    };
    if !supports_tool_choice(capabilities, model).await {
        params.retain(|param| *param != "tool_choice");
    }
    if !supports_stop_reason(model) {
        params.retain(|param| *param != "stop");
    }
    params
}

pub async fn map_params<Capabilities>(
    capabilities: &mut Capabilities,
    request: &mut RequestParams<'_>,
    cost_map: &dyn CostMap,
) -> Result<(), ReasoningError>
where
    Capabilities: ReasoningCapabilities + Send,
{
    if !gpt_reasoning_name(request.model) {
        // the superclass copy reads the plain GPT allowlist, not this provider's filtered one
        base_config().map_params(
            request.model,
            request.non_default_params,
            request.optional_params,
            request.catalog,
        );
        return Ok(());
    }
    let mut wrapped = FoundryCapabilities {
        inner: &mut *capabilities,
        cost_map,
    };
    gpt_5::map_params(
        &mut wrapped,
        &mut Gpt5Mapping {
            model: request.model,
            non_default_params: &mut *request.non_default_params,
            optional_params: &mut *request.optional_params,
            drop_params: request.drop_params,
            catalog: request.catalog,
        },
    )
    .await
}

async fn supports_tool_choice<Capabilities>(capabilities: &mut Capabilities, model: &str) -> bool
where
    Capabilities: ReasoningCapabilities + Send,
{
    let routed = format!("{AZURE_AI_PREFIX}{model}");
    let request = CapabilityRequest {
        model: &routed,
        custom_llm_provider: None,
    };
    capabilities.supports_factory(&request, TOOL_CHOICE).await
}

#[must_use]
pub fn supports_stop_reason(model: &str) -> bool {
    if !model.contains("grok") {
        return true;
    }
    supported_with(model, false).contains(&"stop")
}
