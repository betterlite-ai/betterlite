use std::future::Future;

use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::reasoning_models::ReasoningCapabilities;

pub const GPT5_SERIES_ROUTE: &str = "gpt5_series/";
pub const AZURE_PREFIX: &str = "azure/";
pub const TOOL_CHOICE: &str = "supports_tool_choice";

#[must_use]
pub fn model_map_lookup_name(model: &str) -> String {
    if let Some(routed) = model.strip_prefix(GPT5_SERIES_ROUTE) {
        return format!("{AZURE_PREFIX}{routed}");
    }
    if model.starts_with(AZURE_PREFIX) {
        return model.to_owned();
    }
    format!("{AZURE_PREFIX}{model}")
}

// LiteLLM asks supports_tool_choice with the routing name; every other lookup reads the map key.
fn lookup_model(model: &str, key: &str) -> String {
    if key == TOOL_CHOICE {
        model.to_owned()
    } else {
        model_map_lookup_name(model)
    }
}

pub struct AzureCapabilities<'capabilities, Capabilities> {
    pub inner: &'capabilities mut Capabilities,
}

impl<Capabilities> ReasoningCapabilities for AzureCapabilities<'_, Capabilities>
where
    Capabilities: ReasoningCapabilities + Send,
{
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        async move {
            let model = lookup_model(request.model, key);
            let normalized = CapabilityRequest {
                model: model.as_str(),
                custom_llm_provider: request.custom_llm_provider,
            };
            self.inner.supports_factory(&normalized, key).await
        }
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        async move {
            let model = lookup_model(request.model, key);
            let normalized = CapabilityRequest {
                model: model.as_str(),
                custom_llm_provider: request.custom_llm_provider,
            };
            self.inner.declared_value_factory(&normalized, key).await
        }
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        async move {
            let model = lookup_model(request.model, key);
            let normalized = CapabilityRequest {
                model: model.as_str(),
                custom_llm_provider: request.custom_llm_provider,
            };
            self.inner
                .is_explicitly_disabled_factory(&normalized, key)
                .await
        }
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        async move { self.inner.supports_response_schema(request).await }
    }

    fn catalogue_declares_default(&mut self) -> bool {
        self.inner.catalogue_declares_default()
    }
}
