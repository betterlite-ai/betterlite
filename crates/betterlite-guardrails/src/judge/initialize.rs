use std::sync::Arc;

use betterlite_config::sections::guardrail_judge::{Criterion, JudgeParams};
use rust_decimal::Decimal;

use super::apply::{LLMAsAJudgeGuardrail, OnFailure};
use super::call::JudgeCompletion;
use super::config::{
    GuardrailEntry, ParamKey, boolean_param, criteria_param, get_param, hook_param, number_param,
    text_param,
};
use super::error::JudgeError;
use super::hooks::{EventHookConfig, coerce_event_hook, validate_event_hook};

pub const DEFAULT_THRESHOLD: f64 = 80.0;
pub const DEFAULT_ON_FAILURE: &str = "block";

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<LLMAsAJudgeGuardrail>);
}

struct Resolved {
    guardrail_name: String,
    judge_model: String,
    criteria: Vec<Criterion>,
    on_failure: OnFailure,
    overall_threshold: f64,
    event_hook: EventHookConfig,
    default_on: bool,
}

pub fn initialize_guardrail<R>(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
    completion: Arc<dyn JudgeCompletion>,
    registry: &mut R,
) -> Result<Arc<LLMAsAJudgeGuardrail>, JudgeError>
where
    R: CallbackRegistry,
{
    let resolved = resolve(params, guardrail)?;
    let instance = Arc::new(LLMAsAJudgeGuardrail {
        guardrail_name: resolved.guardrail_name,
        judge_model: resolved.judge_model,
        criteria: resolved.criteria,
        overall_threshold: resolved.overall_threshold,
        on_failure: resolved.on_failure,
        event_hook: resolved.event_hook,
        default_on: resolved.default_on,
        completion,
    });
    registry.add_litellm_callback(Arc::clone(&instance));
    Ok(instance)
}

fn resolve(params: &JudgeParams, guardrail: &GuardrailEntry<'_>) -> Result<Resolved, JudgeError> {
    let Some(guardrail_name) = guardrail.guardrail_name.map(str::to_owned) else {
        return Err(JudgeError::MissingGuardrailName);
    };
    let Some(judge_model) = text_param(get_param(params, guardrail, ParamKey::JudgeModel)?) else {
        return Err(JudgeError::MissingJudgeModel);
    };
    let criteria =
        criteria_param(get_param(params, guardrail, ParamKey::Criteria)?).unwrap_or_default();
    if criteria.is_empty() {
        return Err(JudgeError::MissingCriteria);
    }
    validate_weights(&criteria)?;
    Ok(Resolved {
        guardrail_name,
        judge_model,
        criteria,
        on_failure: resolve_on_failure(params, guardrail)?,
        overall_threshold: number_param(
            get_param(params, guardrail, ParamKey::OverallThreshold)?.as_ref(),
        )
        .unwrap_or(DEFAULT_THRESHOLD),
        event_hook: resolve_event_hook(params, guardrail)?,
        default_on: boolean_param(get_param(params, guardrail, ParamKey::DefaultOn)?.as_ref())
            .unwrap_or(false),
    })
}

fn resolve_on_failure(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
) -> Result<OnFailure, JudgeError> {
    let text = text_param(get_param(params, guardrail, ParamKey::OnFailure)?)
        .unwrap_or_else(|| DEFAULT_ON_FAILURE.to_owned());
    OnFailure::parse(&text).ok_or(JudgeError::InvalidOnFailure { value: text })
}

fn resolve_event_hook(
    params: &JudgeParams,
    guardrail: &GuardrailEntry<'_>,
) -> Result<EventHookConfig, JudgeError> {
    let mode = hook_param(get_param(params, guardrail, ParamKey::Mode)?);
    let event_hook = coerce_event_hook(mode.as_ref())?;
    validate_event_hook(&event_hook)?;
    Ok(event_hook)
}

fn validate_weights(criteria: &[Criterion]) -> Result<(), JudgeError> {
    let mut total = Decimal::ZERO;
    for criterion in criteria {
        let weight = criterion
            .weight
            .to_decimal()
            .ok_or(JudgeError::UnrepresentableWeight)?;
        total = total
            .checked_add(weight)
            .ok_or(JudgeError::WeightOverflow)?;
    }
    let difference = total
        .checked_sub(Decimal::from(100_i64))
        .ok_or(JudgeError::WeightOverflow)?
        .abs();
    if difference > Decimal::new(5, 1) {
        return Err(JudgeError::WeightSum { total });
    }
    Ok(())
}
