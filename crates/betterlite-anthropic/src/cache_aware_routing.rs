pub mod deployment;
pub mod predict_arm;
pub mod services;
pub mod types;

mod cost;
mod count_tokens;
mod fits;
mod from_body;
mod is_warm;
mod minimum_tokens;
mod predict;
mod request_body;
mod supports;
pub(crate) mod tests;
mod validation;

use betterlite_logging::cache_observer::ObservedPrefix;
use betterlite_logging::cache_observer::lookup;
use betterlite_models::anthropic_pricing::ANTHROPIC_PROVIDER;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::PriceLookup;
use chrono::DateTime;
use chrono::Utc;
use http::HeaderMap;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::cache_plan::PromptPrefix;
use crate::cache_plan::parse_prompt;
use crate::cache_prediction_targets::counting::PromptCount;
use crate::cache_prediction_targets::counting::TokenCounter;
use crate::cache_prediction_targets::counting::count_prompt_tokens;
use crate::cache_prediction_targets::headers::supported_prediction_headers;
use crate::cache_prediction_targets::messages_url::MESSAGES_PATH;
use crate::cache_prediction_targets::messages_url::MessagesUrl;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::cache_prediction_targets::target::resolve_prediction_target;
use crate::count_tokens::handler::AnthropicCountTokensHandler;
use crate::count_tokens::handler::CountTokensTransport;

use deployment::ArmRequest;
use deployment::PredictionDeployment;
use deployment::WarmRequest;
use fits::MAX_INPUT_TOKENS;
use fits::MAX_OUTPUT_TOKENS;
use fits::declared_limit;
use fits::resolved_limit;
use from_body::positive_integer;
use request_body::url_path;
use services::CacheCostService;
use services::PredictionServices;
use services::PriceRequest;
use types::CachePredictionArm;
use validation::native_body;

// LiteLLM's frozen dataclass: the predicted body, the prefix parsed from it and the caller's output limit.
#[derive(Clone, Debug)]
pub struct AnthropicCacheRouting {
    pub body: Json,
    pub prefix: PromptPrefix,
    pub requested_output_limit: i64,
}

// The workspace denies multiple_inherent_impl, so this is the type's one impl block and each area's helpers live in its module.
impl AnthropicCacheRouting {
    #[must_use]
    pub fn request_body(
        url: &str,
        headers: &HeaderMap,
        body: &JsonObject,
        request_kwargs: &JsonObject,
        messages: Option<&Json>,
    ) -> Option<Json> {
        if !url_path(url).ends_with(MESSAGES_PATH) || !supported_prediction_headers(headers) {
            return None;
        }
        Some(native_body(body, request_kwargs, messages))
    }

    #[must_use]
    pub fn from_body(body: &Json) -> Option<Self> {
        let text = serde_json::to_string(body).ok()?;
        let prefix = parse_prompt(&text)?;
        let limit = positive_integer(body, "max_tokens")?;
        Some(Self {
            body: body.clone(),
            prefix,
            requested_output_limit: limit,
        })
    }

    #[must_use]
    pub fn supports(
        deployment: &PredictionDeployment<'_>,
        services: &PredictionServices<'_>,
    ) -> bool {
        resolve_prediction_target(
            deployment.litellm_params,
            services.settings,
            services.resolution,
        )
        .is_ok()
    }

    pub async fn is_warm(
        &self,
        request: &WarmRequest<'_>,
        services: &PredictionServices<'_>,
    ) -> bool {
        let Ok(target) = resolve_prediction_target(
            request.deployment.litellm_params,
            services.settings,
            services.resolution,
        ) else {
            return false;
        };
        let scope = cache_scope(
            request.caller_key_hash,
            request.deployment.deployment_id,
            target.api_key.expose_secret(),
            &target.model,
        );
        let observed = ObservedPrefix {
            fingerprint: self.prefix.fingerprint.clone(),
            fingerprints: self.prefix.fingerprints.clone(),
        };
        let observation = lookup(services.cache, &scope, &observed, Some(request.now)).await;
        observation.is_some_and(|found| {
            found.fingerprint == self.prefix.fingerprint && found.expires_at > request.now
        })
    }

    #[must_use]
    pub fn fits(
        deployment: &PredictionDeployment<'_>,
        input_tokens: i64,
        output_tokens: i64,
        services: &PredictionServices<'_>,
        price_lookup: &dyn PriceLookup,
    ) -> bool {
        let Ok(target) = resolve_prediction_target(
            deployment.litellm_params,
            services.settings,
            services.resolution,
        ) else {
            return false;
        };
        let resolved = price_lookup.effective_model_info(&target.model, Some(ANTHROPIC_PROVIDER));
        let limits = (
            declared_limit(deployment.model_info, MAX_INPUT_TOKENS)
                .or_else(|| resolved_limit(resolved.as_ref(), MAX_INPUT_TOKENS)),
            declared_limit(deployment.model_info, MAX_OUTPUT_TOKENS)
                .or_else(|| resolved_limit(resolved.as_ref(), MAX_OUTPUT_TOKENS)),
        );
        let (Some(max_input), Some(max_output)) = limits else {
            return false;
        };
        let Some(window) = input_tokens.checked_add(output_tokens) else {
            return false;
        };
        window <= max_input && output_tokens <= max_output
    }

    pub async fn predict<Counter>(
        &self,
        request: &ArmRequest<'_>,
        counter: &Counter,
        services: &PredictionServices<'_>,
    ) -> CachePredictionArm
    where
        Counter: TokenCounter,
    {
        predict_arm::predict_arm_for_request(request, &self.body, &self.prefix, counter, services)
            .await
    }

    #[must_use]
    pub fn cost(
        service: &dyn CacheCostService,
        arm: &CachePredictionArm,
        output_tokens: i64,
        moment: DateTime<Utc>,
    ) -> Option<Decimal> {
        let scenario = arm.estimate.as_ref()?;
        let request = PriceRequest {
            model: arm.model.as_deref().unwrap_or_default(),
            deployment_id: &arm.deployment_id,
            tokens: &scenario.tokens,
            completion_tokens: output_tokens,
        };
        service.price(&request, moment)
    }

    pub async fn count_tokens<Transport, Builder>(
        handler: &AnthropicCountTokensHandler<Transport>,
        messages_url: &Builder,
        model: &str,
        api_key: &SecretString,
        body: &Json,
    ) -> Option<i64>
    where
        Transport: CountTokensTransport,
        Builder: MessagesUrl,
    {
        count_prompt_tokens(
            handler,
            messages_url,
            &PromptCount {
                model,
                api_key,
                body,
                api_base: None,
            },
        )
        .await
    }
}
