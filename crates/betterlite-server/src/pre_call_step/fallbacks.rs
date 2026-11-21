use std::collections::BTreeSet;
use std::num::FpCategory;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::{PreCall, PreCallError, PreCallInput, PreCallOutput, PreCallStep};
use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;

struct RetryPlan {
    pristine: JsonMap,
    models: Vec<String>,
    original: String,
    rate_limited: PreCallError,
}

#[async_trait]
impl<K: PreCallKey> PreCall<K> for PreCallStep<K> {
    async fn pre_call(&self, input: PreCallInput<'_, K>) -> Result<PreCallOutput, PreCallError> {
        let configured = configured_fallbacks(self, input.key).await;
        let pristine = configured.as_ref().map(|_| input.data.clone());
        let mut tags_before = None;
        match self
            .run_tracked(input.data.clone(), &input, None, &mut tags_before)
            .await
        {
            Ok(output) => Ok(output),
            Err(rate_limited) => {
                let Some(original) =
                    can_fall_back(&rate_limited.data, pristine.as_ref(), configured.as_deref())
                else {
                    return Err(rate_limited);
                };
                let listed = configured.as_deref().unwrap_or_default();
                let Some(models) = fallback_models(self, &original, listed).await else {
                    return Err(rate_limited);
                };
                tracing::info!(
                    "Local rate limit hit for model={original}, attempting fallbacks: {models:?}"
                );
                let Some(held) = pristine else {
                    return Err(rate_limited);
                };
                let plan = RetryPlan {
                    pristine: held,
                    models,
                    original,
                    rate_limited,
                };
                attempt_fallbacks(self, &input, plan, &mut tags_before).await
            }
        }
    }
}

async fn configured_fallbacks<K>(step: &PreCallStep<K>, key: &K) -> Option<Vec<JsonValue>>
where
    K: PreCallKey,
{
    let router = step.ports.router.as_deref()?;
    let listed = match key_fallbacks(key) {
        Some(value) => value.clone(),
        None => JsonValue::Array(router.fallbacks().await?),
    };
    let JsonValue::Array(items) = listed else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    Some(items)
}

async fn fallback_models<K>(
    step: &PreCallStep<K>,
    model: &str,
    fallbacks: &[JsonValue],
) -> Option<Vec<String>>
where
    K: PreCallKey,
{
    let router = step.ports.router.as_deref()?;
    let models = router.fallback_models(model, fallbacks).await?;
    if models.is_empty() {
        return None;
    }
    Some(models)
}

async fn attempt_fallbacks<K>(
    step: &PreCallStep<K>,
    input: &PreCallInput<'_, K>,
    plan: RetryPlan,
    tags_before: &mut Option<BTreeSet<String>>,
) -> Result<PreCallOutput, PreCallError>
where
    K: PreCallKey,
{
    for model in &plan.models {
        if model == &plan.original {
            continue;
        }
        let mut data = plan.pristine.clone();
        data.insert(String::from("model"), JsonValue::String(model.clone()));
        let retry = PreCallInput {
            data: data.clone(),
            request: input.request,
            key: input.key,
            route_type: input.route_type,
            version: input.version,
            model: Some(model.as_str()),
            skip_guardrails: input.skip_guardrails,
        };
        match step
            .run_tracked(data, &retry, Some(plan.original.as_str()), tags_before)
            .await
        {
            Ok(output) => return Ok(output),
            Err(next) => match next.failure {
                PreCallFailure::RateLimited(_) => {}
                failure @ (PreCallFailure::Proxy(_) | PreCallFailure::Http { .. }) => {
                    return Err(PreCallError {
                        failure,
                        data: plan.rate_limited.data.clone(),
                    });
                }
            },
        }
    }
    Err(plan.rate_limited)
}

fn key_fallbacks<K>(key: &K) -> Option<&JsonValue>
where
    K: PreCallKey,
{
    let JsonValue::Object(fields) = key.key_router_settings()? else {
        return None;
    };
    fields.get("fallbacks")
}

fn can_fall_back(
    data: &JsonMap,
    pristine: Option<&JsonMap>,
    configured: Option<&[JsonValue]>,
) -> Option<String> {
    pristine?;
    configured.filter(|items| !items.is_empty())?;
    if enabled(data.get("disable_fallbacks")) {
        return None;
    }
    match data.get("model") {
        Some(JsonValue::String(model)) => Some(model.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}

// LiteLLM reads disable_fallbacks for its Python truthiness, since it is its own flag and not a declared wire field
fn enabled(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(JsonValue::Number(number)) => number
            .as_f64()
            .is_some_and(|amount| !matches!(amount.classify(), FpCategory::Zero)),
        Some(JsonValue::String(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
    }
}
