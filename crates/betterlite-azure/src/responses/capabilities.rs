use std::collections::BTreeSet;
use std::future::Future;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::{CapabilityRequest, NamedCapability};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::error::ResponseParamsError;
use betterlite_openai::response_params::map_openai_params;
use betterlite_openai::response_params::params::{
    ResponsesOptions, ResponsesParams, supported_openai_params,
};

use crate::chat_params::capabilities::AzureCapabilities;

pub const PROVIDER: ProviderId = ProviderId::Azure;
pub const CONTEXT_MANAGEMENT: &str = "context_management";
pub const TEMPERATURE: &str = "temperature";

// Azure drops compaction support, and the O-series deployments also drop temperature
#[must_use]
pub fn supported_params() -> BTreeSet<&'static str> {
    supported_openai_params()
        .into_iter()
        .filter(|param| *param != CONTEXT_MANAGEMENT)
        .collect()
}

#[must_use]
pub fn o_series_supported_params() -> BTreeSet<&'static str> {
    supported_params()
        .into_iter()
        .filter(|param| *param != TEMPERATURE)
        .collect()
}

// Every catalog lookup reads the azure/ entry, which AzureCapabilities supplies
pub struct AzureResponses<'capabilities, Capabilities> {
    inner: AzureCapabilities<'capabilities, Capabilities>,
}

impl<Capabilities> ReasoningCapabilities for AzureResponses<'_, Capabilities>
where
    Capabilities: ResponsesCapabilities + Send,
{
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.inner.supports_factory(request, key)
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        self.inner.declared_value_factory(request, key)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.inner.is_explicitly_disabled_factory(request, key)
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        self.inner.supports_response_schema(request)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        self.inner.catalogue_declares_default()
    }
}

impl<Capabilities> ResponsesCapabilities for AzureResponses<'_, Capabilities>
where
    Capabilities: ResponsesCapabilities + Send,
{
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        self.inner.inner.model_info(model)
    }
}

// The sampling gates read the Azure catalog policy, and only the caller's model text names an error
pub async fn map_params<Capabilities>(
    capabilities: &mut Capabilities,
    options: &ResponsesOptions,
    drop_params: DropParams,
    bundled: &mut BundledReasoningModels,
) -> Result<ResponsesParams, ResponseParamsError>
where
    Capabilities: ResponsesCapabilities + Send,
{
    let mut azure = AzureResponses {
        inner: AzureCapabilities {
            inner: capabilities,
        },
    };
    let request = ResponsesOptions {
        provider: PROVIDER,
        model: options.model.clone(),
        params: options.params.clone(),
    };
    map_openai_params(&mut azure, &request, drop_params, bundled).await
}

// The O-series mapper copies every value and only drops temperature when the caller allows dropping
#[must_use]
pub fn o_series_map_params(
    model: &str,
    params: &ResponsesParams,
    drop_params: DropParams,
) -> ResponsesParams {
    let mut mapped = params.clone();
    if (drop_params.global || drop_params.request) && mapped.temperature.is_some() {
        tracing::debug!(
            "dropping unsupported parameter 'temperature' for Azure OpenAI O-series responses API model {model}"
        );
        mapped.temperature = None;
    }
    mapped
}

// LiteLLM picks the O-series config on the routing name or the catalog's reasoning flag, not on o1/o3/o4
pub async fn is_o_series_model<Capabilities>(capabilities: &mut Capabilities, model: &str) -> bool
where
    Capabilities: ReasoningCapabilities,
{
    if model.to_lowercase().contains("o_series") {
        return true;
    }
    let request = CapabilityRequest {
        model,
        custom_llm_provider: None,
    };
    capabilities
        .supports_factory(&request, NamedCapability::Reasoning.key())
        .await
}
