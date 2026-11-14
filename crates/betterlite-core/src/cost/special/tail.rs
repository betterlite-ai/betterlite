use std::error::Error;
use std::time::SystemTime;

use betterlite_models::billing_text::{
    FormattedPromptData, PromptCallType, count_characters, get_formatted_prompt,
    get_response_string,
};
use betterlite_models::provider_model_sets::ProviderModelSet;
use rust_decimal::Decimal;

use super::error::SpecialChargeError;
use super::replicate::replicate_pricing;
use super::request::{SpecialOutcome, SpecialRequest, SpecialServices, TokenStage};
use crate::cost::context::response_time::response_time_ms;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{AttemptBilling, AttemptOutcome, PreparedBillingAttempt};

// Exact vertex_ai prices by characters, so the prompt and a typed completion are counted here.
fn record_vertex_characters<ImageError>(
    request: &SpecialRequest<'_>,
    billing: &mut AttemptBilling,
) -> Result<(), SpecialChargeError<ImageError>>
where
    ImageError: Error + 'static,
{
    if !request.messages.is_empty() {
        let prompt = get_formatted_prompt(
            &FormattedPromptData {
                messages: Some(request.messages),
                prompt: None,
                input: None,
            },
            PromptCallType::Completion,
        )?;
        billing.prompt_characters =
            Some(u64::try_from(count_characters(&prompt)).unwrap_or(u64::MAX));
    }
    if let Some(response) = request.response.model_response {
        let text = get_response_string(response)?;
        billing.completion_characters =
            Some(u64::try_from(count_characters(&text)).unwrap_or(u64::MAX));
    }
    Ok(())
}

// The legacy Together category replaces the model only when the registry has no price for it.
fn together_category<ImagePolicy, Resolver>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    attempt: &PreparedBillingAttempt,
    model: &str,
) -> Result<Option<String>, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
{
    let named = model.contains("togethercomputer")
        || model.contains("together_ai")
        || attempt.custom_llm_provider.as_deref() == Some("together_ai");
    if !named
        || services
            .special_policy
            .has_together_registry_pricing(model, services.lookup)?
    {
        return Ok(None);
    }
    let category = services
        .special_policy
        .together_category(model, request.call_type)?;
    Ok(Some(category))
}

// A Replicate model without a catalog entry is billed by the time the request ran.
fn unmapped_replicate<ImagePolicy, Resolver, Clock>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    model: &str,
    timing: (Decimal, &mut Clock),
) -> Result<Option<Decimal>, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Clock: FnMut() -> SystemTime,
{
    let in_replicate_set = services
        .replicate_models
        .sets
        .get(&ProviderModelSet::Replicate)
        .is_some_and(|names| names.contains(model));
    if !(in_replicate_set || model.contains("replicate"))
        || services.lookup.raw_cost_map_entry(model).is_some()
    {
        return Ok(None);
    }
    let (response_ms, clock) = timing;
    let charge = replicate_pricing(
        services.replicate_price,
        response_ms,
        request.response.replicate,
        clock,
    )?;
    Ok(Some(charge))
}

// Everything after the early returns: response time, the Together category, unmapped Replicate and the Vertex character counts.
pub(super) fn token_tail<Resolver, ImagePolicy, Clock>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    mut attempt: PreparedBillingAttempt,
    clock: &mut Clock,
) -> Result<SpecialOutcome, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Clock: FnMut() -> SystemTime,
{
    let response_ms = response_time_ms(
        request.response.cost_response,
        request.total_time,
        request.logging.timing.as_ref(),
    )?;
    let model = attempt
        .model
        .clone()
        .ok_or(SpecialChargeError::ModelMissing)?;
    match together_category(request, services, &attempt, &model)? {
        Some(category) => attempt.model = Some(category),
        None => {
            if let Some(charge) =
                unmapped_replicate(request, services, &model, (response_ms, clock))?
            {
                return Ok(SpecialOutcome::Charged(charge));
            }
        }
    }
    if attempt.custom_llm_provider.as_deref() == Some("vertex_ai")
        && let AttemptOutcome::Ready(billing) = &mut attempt.outcome
    {
        record_vertex_characters(request, billing)?;
    }
    Ok(SpecialOutcome::Continue(Box::new(TokenStage {
        attempt,
        response_time_ms: response_ms,
        request_model_for_cost: request.logging.model.map(str::to_owned),
    })))
}
