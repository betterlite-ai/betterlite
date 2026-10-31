use std::cmp::Reverse;

use rust_decimal::Decimal;

use super::rates::{cost_per_unit, is_present, rate, service_tier_cost_key};
use super::reasoning::resolve_billed_reasoning_rate;
use super::threshold::{parse_token_threshold, prompt_exceeds};
use super::{EffectiveRates, ModelInfo, ModelPricing, RequestContext, TokenRateError, Usage};
use crate::off_peak::{self, BaseCosts, ResolvedBaseCosts};
use crate::tiered_pricing::{self, Tier};

const INPUT_THRESHOLD_PREFIX: &str = "input_cost_per_token_above_";
const NON_STANDARD_SUFFIXES: [&str; 7] = [
    "_ultrafast",
    "_balanced",
    "_priority",
    "_auto",
    "_flex",
    "_fast",
    "_batches",
];

pub fn get_token_base_cost(
    pricing: &ModelPricing<'_>,
    usage: Usage,
    context: &RequestContext<'_>,
) -> Result<EffectiveRates, TokenRateError> {
    let selected_tier =
        tiered_pricing::select_tier_for_input(pricing.tiered_pricing, usage.prompt_tokens)?
            .filter(|tier| tier.input_token.is_some());
    selected_tier.map_or_else(
        || standard_rates(pricing, usage, context),
        |tier| tiered_rates(pricing, tier, context),
    )
}

// One tier is picked and every undeclared rate falls back to the tier's input rate; output falls back to the model's own output rate instead.
fn tiered_rates(
    pricing: &ModelPricing<'_>,
    tier: &Tier,
    context: &RequestContext<'_>,
) -> Result<EffectiveRates, TokenRateError> {
    let model = pricing.model;
    let input = tiered_pricing::tier_rate(tier.input_token.as_ref(), None)?;
    let output = if tier.output_token.is_some() {
        tiered_pricing::tier_rate(tier.output_token.as_ref(), None)?
    } else {
        cost_per_unit(model, "output_cost_per_token", Some(Decimal::ZERO))?.unwrap_or(Decimal::ZERO)
    };
    let cache_creation = tiered_pricing::tier_rate(
        tier.cache_creation_token_cost.as_ref(),
        tier.input_token.as_ref(),
    )?;
    // The one-hour fallback reads truthiness: an explicit zero here still yields cache_creation.
    let above_1hr_raw = tiered_pricing::tier_rate(
        tier.cache_creation_above_1hr_cost.as_ref(),
        tier.cache_creation_token_cost.as_ref(),
    )?;
    let above_1hr = if above_1hr_raw.is_zero() {
        cache_creation
    } else {
        above_1hr_raw
    };
    let cache_read = tiered_pricing::tier_rate(
        tier.cache_read_input_token_cost.as_ref(),
        tier.input_token.as_ref(),
    )?;
    finish_rates(
        pricing,
        context,
        ResolvedBaseCosts {
            prompt_rate: input,
            completion_rate: output,
            cache_creation_rate: cache_creation,
            cache_creation_above_1hr_rate: above_1hr,
            cache_read_rate: cache_read,
        },
        tiered_reasoning_rate(tier)?,
    )
}

fn finish_rates(
    pricing: &ModelPricing<'_>,
    context: &RequestContext<'_>,
    resolved: ResolvedBaseCosts,
    tier_reasoning: Option<Decimal>,
) -> Result<EffectiveRates, TokenRateError> {
    let reasoning = resolve_billed_reasoning_rate(
        pricing.model,
        tier_reasoning,
        pricing.off_peak_pricing,
        context,
        resolved.completion_rate,
    )?;
    Ok(EffectiveRates {
        input_rate: resolved.prompt_rate,
        output_rate: resolved.completion_rate,
        cache_read_rate: resolved.cache_read_rate,
        cache_creation_rate: resolved.cache_creation_rate,
        cache_creation_above_1hr_rate: resolved.cache_creation_above_1hr_rate,
        reasoning_rate: reasoning,
    })
}

fn tiered_reasoning_rate(tier: &Tier) -> Result<Option<Decimal>, TokenRateError> {
    if tier.output_reasoning_token.is_none() && tier.output_token.is_none() {
        return Ok(None);
    }
    Ok(Some(tiered_pricing::tier_rate(
        tier.output_reasoning_token.as_ref(),
        tier.output_token.as_ref(),
    )?))
}

struct BaseRates {
    prompt: Decimal,
    completion: Decimal,
    cache_creation: Option<Decimal>,
    cache_creation_above_1hr: Option<Decimal>,
    cache_read: Option<Decimal>,
}

fn standard_rates(
    pricing: &ModelPricing<'_>,
    usage: Usage,
    context: &RequestContext<'_>,
) -> Result<EffectiveRates, TokenRateError> {
    let model = pricing.model;
    let service_tier = context.service_tier;
    let base = initial_base_rates(model, service_tier)?;
    let threshold_text = crossed_input_threshold(model, usage, context, service_tier)?;
    let overridden = match threshold_text {
        Some(text) => apply_threshold_overrides(model, service_tier, &text, &base)?,
        None => base,
    };

    let resolved = resolve_standard_cost(pricing, context, &overridden)?;
    finish_rates(pricing, context, resolved, None)
}

fn initial_base_rates(
    model: &ModelInfo,
    service_tier: Option<&str>,
) -> Result<BaseRates, TokenRateError> {
    let input_key = service_tier_cost_key("input_cost_per_token", service_tier);
    let output_key = service_tier_cost_key("output_cost_per_token", service_tier);
    let cache_key = service_tier_cost_key("cache_creation_input_token_cost", service_tier);
    let read_key = service_tier_cost_key("cache_read_input_token_cost", service_tier);

    let prompt = rate(model, &input_key, Decimal::ZERO)?;
    let mut completion = rate(model, &output_key, Decimal::ZERO)?;
    if completion.is_zero()
        && let Some(image_rate) = cost_per_unit(model, "output_cost_per_image_token", None)?
    {
        completion = image_rate;
    }
    Ok(BaseRates {
        prompt,
        completion,
        cache_creation: cost_per_unit(model, &cache_key, None)?,
        cache_creation_above_1hr: cost_per_unit(
            model,
            "cache_creation_input_token_cost_above_1hr",
            None,
        )?,
        cache_read: cost_per_unit(model, &read_key, None)?,
    })
}

fn resolve_standard_cost(
    pricing: &ModelPricing<'_>,
    context: &RequestContext<'_>,
    base: &BaseRates,
) -> Result<ResolvedBaseCosts, TokenRateError> {
    let input_rate_for_missing_cache =
        off_peak_input_rate_fallback(pricing.off_peak_pricing, context, base.prompt)?;
    let base_costs = BaseCosts {
        prompt_rate: base.prompt,
        completion_rate: base.completion,
        cache_creation_rate: base.cache_creation.unwrap_or(input_rate_for_missing_cache),
        cache_creation_above_1hr_rate: base.cache_creation_above_1hr,
        cache_read_rate: base.cache_read.unwrap_or(input_rate_for_missing_cache),
    };
    Ok(off_peak::apply_off_peak_to_base_costs(
        pricing.off_peak_pricing,
        context.moment,
        base_costs,
        context.database,
    )?)
}

fn off_peak_input_rate_fallback(
    off_peak_pricing: Option<&off_peak::schema::OffPeakBlock>,
    context: &RequestContext<'_>,
    prompt_rate: Decimal,
) -> Result<Decimal, TokenRateError> {
    let probe = off_peak::TokenRates {
        input_rate: prompt_rate,
        output_rate: Decimal::ZERO,
        cache_read_rate: Decimal::ZERO,
        cache_creation_rate: Decimal::ZERO,
        reasoning_rate: None,
    };
    let resolved = off_peak::apply_off_peak_pricing(
        off_peak_pricing,
        context.moment,
        probe,
        context.database,
    )?;
    Ok(resolved.input_rate)
}

fn apply_threshold_overrides(
    model: &ModelInfo,
    service_tier: Option<&str>,
    threshold_text: &str,
    rates: &BaseRates,
) -> Result<BaseRates, TokenRateError> {
    let keyed = |prefix: &str| {
        service_tier_cost_key(
            &format!("{prefix}_above_{threshold_text}_tokens"),
            service_tier,
        )
    };
    let prompt = cost_per_unit(model, &keyed("input_cost_per_token"), Some(rates.prompt))?
        .unwrap_or(rates.prompt);
    let completion = cost_per_unit(
        model,
        &keyed("output_cost_per_token"),
        Some(rates.completion),
    )?
    .unwrap_or(rates.completion);
    let cache_creation = cost_per_unit(
        model,
        &keyed("cache_creation_input_token_cost"),
        rates.cache_creation,
    )?;
    let cache_creation_above_1hr = cost_per_unit(
        model,
        &keyed("cache_creation_input_token_cost_above_1hr"),
        rates.cache_creation_above_1hr,
    )?;
    let cache_read = cost_per_unit(
        model,
        &keyed("cache_read_input_token_cost"),
        rates.cache_read,
    )?;
    Ok(BaseRates {
        prompt,
        completion,
        cache_creation,
        cache_creation_above_1hr,
        cache_read,
    })
}

fn crossed_input_threshold(
    model: &ModelInfo,
    usage: Usage,
    context: &RequestContext<'_>,
    service_tier: Option<&str>,
) -> Result<Option<String>, TokenRateError> {
    let tier_suffix = service_tier_cost_key("", service_tier);
    let inclusive = context.provider == Some("xai");
    let mut parsed = Vec::new();
    for key in model.keys() {
        if !is_threshold_candidate(key, &tier_suffix) {
            continue;
        }
        let text = above_text(key);
        let limit = parse_token_threshold(text, key)?;
        parsed.push((limit, key.clone(), text.to_owned()));
    }
    parsed.sort_by_key(|(limit, _, _)| Reverse(*limit));
    for (limit, key, text) in parsed {
        if !is_present(model, &key) {
            continue;
        }
        if prompt_exceeds(usage.prompt_tokens, limit, inclusive) {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

fn is_threshold_candidate(key: &str, tier_suffix: &str) -> bool {
    if !key.starts_with(INPUT_THRESHOLD_PREFIX) {
        return false;
    }
    let is_non_standard = NON_STANDARD_SUFFIXES
        .iter()
        .any(|suffix| key.ends_with(suffix));
    !is_non_standard || (!tier_suffix.is_empty() && key.ends_with(tier_suffix))
}

// A key without `_tokens` at all is used in full, matching the source's lenient split.
fn above_text(key: &str) -> &str {
    let rest = key.strip_prefix(INPUT_THRESHOLD_PREFIX).unwrap_or(key);
    rest.split("_tokens").next().unwrap_or(rest)
}
