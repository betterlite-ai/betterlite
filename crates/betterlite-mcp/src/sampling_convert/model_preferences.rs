use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Deserialize;

use super::defaults::null_as_default;
use super::error::SamplingConvertError;

// 0.5: a metric with no spread scores every candidate the same
const NEUTRAL_SCORE: Decimal = Decimal::from_parts(5, 0, 0, false, 1);

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ModelPreferences {
    #[serde(default, deserialize_with = "null_as_default")]
    pub hints: Vec<ModelHint>,
    #[serde(
        default,
        rename = "costPriority",
        alias = "cost_priority",
        deserialize_with = "null_as_default"
    )]
    pub cost_priority: Decimal,
    #[serde(
        default,
        rename = "speedPriority",
        alias = "speed_priority",
        deserialize_with = "null_as_default"
    )]
    pub speed_priority: Decimal,
    #[serde(
        default,
        rename = "intelligencePriority",
        alias = "intelligence_priority",
        deserialize_with = "null_as_default"
    )]
    pub intelligence_priority: Decimal,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct ModelHint {
    #[serde(default, deserialize_with = "null_as_default")]
    pub name: String,
}

// the raw fields LiteLLM reads from a model catalog entry; scoring resolves their defaults
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelInfo {
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub input_cost_per_token: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub output_cost_per_token: Option<Decimal>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub output_tokens_per_second: Option<Decimal>,
}

#[must_use]
pub fn has_priorities(preferences: &ModelPreferences) -> bool {
    [
        preferences.cost_priority,
        preferences.speed_priority,
        preferences.intelligence_priority,
    ]
    .into_iter()
    .any(|weight| weight > Decimal::ZERO)
}

struct ScoredModel {
    name: String,
    cost: Decimal,
    max_output_tokens: u64,
    output_tokens_per_second: Decimal,
}

struct MetricValues {
    costs: Vec<Decimal>,
    max_outputs: Vec<Decimal>,
    throughputs: Vec<Decimal>,
}

fn scored_models(
    model_names: &[String],
    model_info: &BTreeMap<String, ModelInfo>,
) -> Result<Vec<ScoredModel>, SamplingConvertError> {
    let mut scored = Vec::new();
    for name in model_names {
        let Some(info) = model_info.get(name) else {
            continue;
        };
        scored.push(ScoredModel {
            name: name.clone(),
            cost: info
                .input_cost_per_token
                .unwrap_or_default()
                .checked_add(info.output_cost_per_token.unwrap_or_default())
                .ok_or(SamplingConvertError::PriorityScoreOverflow)?,
            max_output_tokens: info
                .max_output_tokens
                .filter(|value| *value != 0)
                .or(info.max_tokens)
                .unwrap_or(0),
            output_tokens_per_second: info.output_tokens_per_second.unwrap_or_default(),
        });
    }
    Ok(scored)
}

fn metric_values(scored: &[ScoredModel]) -> MetricValues {
    let mut costs = Vec::with_capacity(scored.len());
    let mut max_outputs = Vec::with_capacity(scored.len());
    let mut throughputs = Vec::with_capacity(scored.len());
    for entry in scored {
        costs.push(entry.cost);
        max_outputs.push(Decimal::from(entry.max_output_tokens));
        throughputs.push(entry.output_tokens_per_second);
    }
    MetricValues {
        costs,
        max_outputs,
        throughputs,
    }
}

pub fn select_model_by_priority(
    model_names: &[String],
    preferences: &ModelPreferences,
    model_info: &BTreeMap<String, ModelInfo>,
) -> Result<Option<String>, SamplingConvertError> {
    let scored = scored_models(model_names, model_info)?;
    if scored.is_empty() {
        return Ok(None);
    }
    let values = metric_values(&scored);
    let cost_scores = normalise(&values.costs, true)?;
    // LiteLLM keeps every candidate neutral when no model reports a throughput
    let speed_scores = if values
        .throughputs
        .iter()
        .all(|value| *value <= Decimal::ZERO)
    {
        vec![NEUTRAL_SCORE; scored.len()]
    } else {
        normalise(&values.throughputs, false)?
    };
    let intelligence_scores = normalise(&values.max_outputs, false)?;
    let mut best_name = None;
    let mut best_score = Decimal::NEGATIVE_ONE;
    let ranked = scored
        .iter()
        .zip(cost_scores)
        .zip(speed_scores)
        .zip(intelligence_scores);
    for (((entry, cost_score), speed_score), intelligence_score) in ranked {
        let score = weighted_score(preferences, cost_score, speed_score, intelligence_score)?;
        if score > best_score {
            best_score = score;
            best_name = Some(entry.name.clone());
        }
    }
    Ok(best_name)
}

fn weighted_score(
    preferences: &ModelPreferences,
    cost_score: Decimal,
    speed_score: Decimal,
    intelligence_score: Decimal,
) -> Result<Decimal, SamplingConvertError> {
    let weighted = [
        (preferences.cost_priority, cost_score),
        (preferences.speed_priority, speed_score),
        (preferences.intelligence_priority, intelligence_score),
    ];
    let mut total = Decimal::ZERO;
    for (weight, score) in weighted {
        let contribution = weight
            .checked_mul(score)
            .ok_or(SamplingConvertError::PriorityScoreOverflow)?;
        total = total
            .checked_add(contribution)
            .ok_or(SamplingConvertError::PriorityScoreOverflow)?;
    }
    Ok(total)
}

pub(crate) fn normalise(
    values: &[Decimal],
    invert: bool,
) -> Result<Vec<Decimal>, SamplingConvertError> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let lowest = values.iter().copied().min().unwrap_or(Decimal::ZERO);
    let highest = values.iter().copied().max().unwrap_or(Decimal::ZERO);
    if highest == lowest {
        return Ok(vec![NEUTRAL_SCORE; values.len()]);
    }
    let spread = highest
        .checked_sub(lowest)
        .ok_or(SamplingConvertError::PriorityScoreOverflow)?;
    let mut scores = Vec::with_capacity(values.len());
    for value in values {
        let offset = value
            .checked_sub(lowest)
            .ok_or(SamplingConvertError::PriorityScoreOverflow)?;
        let ratio = offset
            .checked_div(spread)
            .ok_or(SamplingConvertError::PriorityScoreOverflow)?;
        let score = if invert {
            Decimal::ONE
                .checked_sub(ratio)
                .ok_or(SamplingConvertError::PriorityScoreOverflow)?
        } else {
            ratio
        };
        scores.push(score);
    }
    Ok(scores)
}

fn resolve_hint(hints: &[ModelHint], available_model_names: &[String]) -> Option<String> {
    for hint in hints {
        if hint.name.is_empty() {
            continue;
        }
        let hint_name = hint.name.as_str();
        let direct = available_model_names
            .iter()
            .find(|model| model.as_str() == hint_name);
        if let Some(model) = direct {
            return Some(model.clone());
        }
        let lowered = hint_name.to_lowercase();
        let substring = available_model_names
            .iter()
            .find(|model| model.to_lowercase().contains(&lowered));
        if let Some(model) = substring {
            return Some(model.clone());
        }
    }
    None
}

pub fn resolve_model_from_preferences(
    preferences: Option<&ModelPreferences>,
    available_model_names: &[String],
    default_model: Option<&str>,
    fallback_sampling_model: Option<&str>,
    model_info: &BTreeMap<String, ModelInfo>,
) -> Result<String, SamplingConvertError> {
    if let Some(given) = preferences {
        let hinted = resolve_hint(&given.hints, available_model_names);
        if let Some(model) = hinted {
            return Ok(model);
        }
        if has_priorities(given)
            && let Some(best) = select_model_by_priority(available_model_names, given, model_info)?
        {
            return Ok(best);
        }
    }
    if let Some(given_default) = default_model.filter(|model| !model.is_empty()) {
        return Ok(given_default.to_owned());
    }
    if let Some(first) = available_model_names.first() {
        return Ok(first.clone());
    }
    if let Some(fallback) = fallback_sampling_model.filter(|model| !model.is_empty()) {
        return Ok(fallback.to_owned());
    }
    Err(SamplingConvertError::NoModelResolved)
}
