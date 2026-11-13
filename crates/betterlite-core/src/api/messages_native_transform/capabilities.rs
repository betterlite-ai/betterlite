use std::future::Future;

use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_anthropic::anthropic_capability_catalog::thinking::{
    ThinkingError, ThinkingRequest, maybe_drop_disabled_thinking, translate_legacy_thinking,
};
use betterlite_anthropic::anthropic_capability_catalog::{
    CatalogSources, EffortThresholds, OptionalParams,
};
use betterlite_anthropic::capabilities::effort::validate_effort_for_model;
use betterlite_anthropic::capabilities::effort_param::model_supports_effort_param;
use betterlite_anthropic::capabilities::reasoning::{
    ReasoningEffortError, ReasoningRequest, map_reasoning_effort,
};
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

pub const ADAPTIVE_THINKING: &str = "supports_adaptive_thinking";
pub const REASONING: &str = "supports_reasoning";

pub type Thinking = IndexMap<String, Json>;

pub struct LegacyShapes<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub params: &'request mut OptionalParams,
}

fn thinking_fields(value: Option<Json>) -> Option<Thinking> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    }
}

// The capability questions the request transform asks; production answers them through B569.
pub trait ThinkingCapabilities {
    fn supports(
        &mut self,
        model: &str,
        key: &str,
        provider: &str,
    ) -> impl Future<Output = bool> + Send;

    fn map_effort<'seam>(
        &'seam mut self,
        model: &'seam str,
        effort: &'seam str,
        provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam;

    fn validate_effort(
        &mut self,
        model: &str,
        effort: Option<&str>,
        provider: &str,
    ) -> impl Future<Output = Option<String>> + Send;

    fn supports_effort_param(
        &mut self,
        model: &str,
        provider: &str,
    ) -> impl Future<Output = bool> + Send;

    fn drop_disabled_thinking(
        &mut self,
        model: &str,
        params: &mut OptionalParams,
        provider: &str,
    ) -> impl Future<Output = bool> + Send;

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam;
}

pub struct AnthropicCapabilities<'seam, 'resources, Source, Transport> {
    pub environment: &'seam mut CapabilityEnvironment<'resources, Source, Transport>,
    pub catalogs: &'seam CatalogSources<'resources>,
    pub budgets: &'seam Settings,
    // LiteLLM tags the mapped-effort failure with the routed provider, then discards that tag.
    pub llm_provider: &'seam str,
}

impl<Source, Transport> ThinkingCapabilities for AnthropicCapabilities<'_, '_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    fn supports(
        &mut self,
        model: &str,
        key: &str,
        provider: &str,
    ) -> impl Future<Output = bool> + Send {
        supports_model_capability(model, key, provider, self.environment, self.catalogs)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        model: &'seam str,
        effort: &'seam str,
        provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        let request = ReasoningRequest {
            reasoning_effort: Some(effort),
            model,
            custom_llm_provider: provider,
            llm_provider: self.llm_provider,
            budgets: self.budgets,
        };
        async move {
            let mapped = map_reasoning_effort(&request, self.environment, self.catalogs).await?;
            Ok(thinking_fields(mapped))
        }
    }

    fn validate_effort(
        &mut self,
        model: &str,
        effort: Option<&str>,
        provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        validate_effort_for_model(model, effort, provider, self.environment, self.catalogs)
    }

    fn supports_effort_param(
        &mut self,
        model: &str,
        provider: &str,
    ) -> impl Future<Output = bool> + Send {
        model_supports_effort_param(model, provider, self.environment, self.catalogs)
    }

    fn drop_disabled_thinking(
        &mut self,
        model: &str,
        params: &mut OptionalParams,
        provider: &str,
    ) -> impl Future<Output = bool> + Send {
        maybe_drop_disabled_thinking(model, params, provider, self.environment, self.catalogs)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        let thresholds = EffortThresholds {
            medium: self.budgets.medium,
            high: self.budgets.high,
            xhigh: self.budgets.xhigh,
        };
        async move {
            let thinking = ThinkingRequest {
                model: request.model,
                custom_llm_provider: request.custom_llm_provider,
                thresholds,
            };
            translate_legacy_thinking(&thinking, request.params, self.environment, self.catalogs)
                .await
        }
    }
}
