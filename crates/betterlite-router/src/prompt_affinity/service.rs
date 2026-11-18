use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::message_utilities::prompt_cache_checks::{
    DEFAULT_MIN_CACHE_TOKENS, PromptCacheRequest, PromptCacheSettings, get_prompt_cache_min_tokens,
    is_prompt_caching_valid_prompt,
};
use betterlite_logging::logging_metadata::base64::Base64Truncator;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::cache_defaults::chat::{RouterProjection, messages_with_defaults};
use betterlite_prompts::cache_defaults::{CacheFacts, CachePolicy};
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::Counter;
use thiserror::Error;

use crate::deployment_identity::spec::DeploymentSpec;

use super::cache::{PinCache, pinned_model_id, store_pin};
use super::json::{messages_json, text};
use super::prefix::PrefixError;

// the call types PromptCachingDeploymentCheck.async_log_success_event pins a deployment for
const PINNED_CALL_TYPES: [&str; 3] = ["completion", "acompletion", "anthropic_messages"];

#[derive(Debug, Error)]
pub enum PromptAffinityError {
    #[error(transparent)]
    Prefix(#[from] PrefixError),
}

// litellm.utils.is_prompt_caching_valid_prompt, injected so the deployment check runs without a tokenizer
#[async_trait]
pub trait TokenGate: Send + Sync {
    async fn meets_minimum(&self, model: &str, messages: &[Json], minimum: Option<u64>) -> bool;
}

pub struct CountingGate {
    pub counter: Arc<Counter>,
    pub settings: PromptCacheSettings,
    pub catalog: Arc<ModelCatalog>,
}

#[async_trait]
impl TokenGate for CountingGate {
    async fn meets_minimum(&self, model: &str, messages: &[Json], minimum: Option<u64>) -> bool {
        let request = PromptCacheRequest {
            model,
            messages: Some(messages),
            tools: None,
            custom_llm_provider: None,
            min_token_count: minimum,
        };
        is_prompt_caching_valid_prompt(&request, self.settings, &self.counter, &self.catalog).await
    }
}

// the globals AnthropicCacheControlHook reads when it reproduces the default breakpoints
#[derive(Clone, Debug, Default)]
pub struct InjectionDefaults {
    pub global_enabled: bool,
    pub ttl: Option<String>,
    pub alias_map: BTreeMap<String, String>,
}

pub struct PromptAffinityDependencies {
    pub pins: Arc<dyn PinCache>,
    pub gate: Arc<dyn TokenGate>,
    pub facts: Arc<dyn CacheFacts + Send + Sync>,
    pub catalog: Arc<ModelCatalog>,
    pub settings: PromptCacheSettings,
    pub truncator: Base64Truncator,
    pub defaults: InjectionDefaults,
}

pub struct FilterRequest<'request> {
    pub model: &'request str,
    pub deployments: &'request [DeploymentSpec],
    pub messages: Option<&'request [Message]>,
    pub request_kwargs: Option<&'request Object>,
    pub priority_group: bool,
    pub target_order: Option<i64>,
}

pub struct SuccessEvent<'event> {
    pub call_type: &'event str,
    pub model: &'event str,
    pub messages: Option<&'event [Json]>,
    pub model_id: Option<&'event str>,
}

// PromptCachingDeploymentCheck: narrow a model group to the deployment that served this prefix
pub struct PromptAffinity {
    dependencies: PromptAffinityDependencies,
}

impl From<PromptAffinityDependencies> for PromptAffinity {
    fn from(dependencies: PromptAffinityDependencies) -> Self {
        Self { dependencies }
    }
}

impl PromptAffinity {
    // PromptCachingDeploymentCheck.async_filter_deployments
    pub async fn filter_deployments(
        &self,
        request: &FilterRequest<'_>,
    ) -> Result<Vec<DeploymentSpec>, PromptAffinityError> {
        let pool = request.deployments;
        if request.priority_group || request.target_order.is_some() {
            return Ok(pool.to_vec());
        }
        let Some(messages) = request.messages else {
            return Ok(pool.to_vec());
        };
        let minimum = self.minimum_tokens(pool);
        let counted = self
            .dependencies
            .gate
            .meets_minimum(request.model, &messages_json(messages), Some(minimum))
            .await;
        if !counted {
            return Ok(pool.to_vec());
        }
        let models = deployment_models(pool);
        let empty = Object::new();
        let request_kwargs = request.request_kwargs.unwrap_or(&empty);
        let injected = messages_with_defaults(
            &self.projection(request, messages, &models, request_kwargs),
            self.dependencies.facts.as_ref(),
        );
        let affinity = messages_json(&injected);
        let Some(model_id) = self.pin_for(Some(&affinity), None).await? else {
            return Ok(pool.to_vec());
        };
        Ok(select_deployment(pool, &model_id))
    }

    // PromptCachingDeploymentCheck.async_log_success_event
    pub async fn record_success(
        &self,
        event: &SuccessEvent<'_>,
    ) -> Result<(), PromptAffinityError> {
        if !PINNED_CALL_TYPES.contains(&event.call_type) {
            return Ok(());
        }
        let (Some(messages), Some(model_id)) = (event.messages, event.model_id) else {
            return Ok(());
        };
        let counted = self
            .dependencies
            .gate
            .meets_minimum(event.model, messages, None)
            .await;
        if !counted {
            return Ok(());
        }
        store_pin(
            self.dependencies.pins.as_ref(),
            model_id,
            Some(messages),
            None,
            &self.dependencies.truncator,
        )
        .await?;
        Ok(())
    }

    // PromptCachingCache.async_get_model_id: the pinned deployment for these messages, if any
    pub async fn pin_for(
        &self,
        messages: Option<&[Json]>,
        tools: Option<&[Json]>,
    ) -> Result<Option<String>, PromptAffinityError> {
        Ok(pinned_model_id(
            self.dependencies.pins.as_ref(),
            messages,
            tools,
            &self.dependencies.truncator,
        )
        .await?)
    }

    fn minimum_tokens(&self, pool: &[DeploymentSpec]) -> u64 {
        pool.iter()
            .filter_map(|deployment| deployment_model(deployment))
            .map(|model| {
                get_prompt_cache_min_tokens(
                    model,
                    self.dependencies.settings,
                    &self.dependencies.catalog,
                )
            })
            .min()
            .unwrap_or(DEFAULT_MIN_CACHE_TOKENS)
    }

    fn projection<'projection>(
        &'projection self,
        request: &'projection FilterRequest<'_>,
        messages: &'projection [Message],
        models: &'projection [String],
        request_kwargs: &'projection Object,
    ) -> RouterProjection<'projection> {
        RouterProjection {
            messages,
            models,
            tools: tools_of(request.request_kwargs),
            enable_prompt_caching: Some(enable_flag(request.request_kwargs)),
            request_kwargs,
            alias_map: &self.dependencies.defaults.alias_map,
            policy: CachePolicy {
                global_enabled: self.dependencies.defaults.global_enabled,
                ttl: self.dependencies.defaults.ttl.as_deref(),
            },
        }
    }
}

// Router.async_callback_filter_deployments only ever pins a deployment already in the pool
fn select_deployment(pool: &[DeploymentSpec], model_id: &str) -> Vec<DeploymentSpec> {
    pool.iter()
        .find(|deployment| deployment.id() == Some(model_id))
        .map_or_else(|| pool.to_vec(), |deployment| vec![deployment.clone()])
}

fn deployment_models(pool: &[DeploymentSpec]) -> Vec<String> {
    pool.iter()
        .filter_map(|deployment| deployment_model(deployment))
        .map(str::to_owned)
        .collect()
}

fn deployment_model(deployment: &DeploymentSpec) -> Option<&str> {
    deployment
        .litellm_params
        .get("model")
        .and_then(text)
        .filter(|model| !model.is_empty())
}

fn tools_of(request_kwargs: Option<&Object>) -> Option<&[PromptJson]> {
    match request_kwargs?.get("tools") {
        Some(PromptJson::List(tools)) => Some(tools),
        _ => None,
    }
}

fn enable_flag(request_kwargs: Option<&Object>) -> bool {
    request_kwargs.is_some_and(|kwargs| {
        matches!(
            kwargs.get("enable_prompt_caching"),
            Some(PromptJson::Bool(true))
        )
    })
}
