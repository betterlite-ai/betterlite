use crate::cache_accounting::dialect::{
    DialectInputs, ModelFacts, supports_openai_breakpoint, targets_openai_breakpoint,
};
use crate::cache_accounting::points::CacheControlInjectionPoint;

// Stamped only for a model that speaks the OpenAI dialect at all, so a point keeps its shape for every other model.
#[must_use]
pub fn stamped_with_dialect(
    points: &[CacheControlInjectionPoint],
    inputs: &DialectInputs<'_>,
    facts: &dyn ModelFacts,
) -> Vec<CacheControlInjectionPoint> {
    if !supports_openai_breakpoint(inputs.model, facts) {
        return points.to_vec();
    }
    let dialect = targets_openai_breakpoint(inputs, facts);
    points
        .iter()
        .map(|point| point.restamped(Some(dialect), None))
        .collect()
}

// A zero external count leaves the stamp off, since an absent stamp already reads as zero downstream.
#[must_use]
pub fn stamped_for_prompt_hook(
    points: &[CacheControlInjectionPoint],
    external_breakpoints: usize,
    inputs: &DialectInputs<'_>,
    facts: &dyn ModelFacts,
) -> Vec<CacheControlInjectionPoint> {
    let stamped = stamped_with_dialect(points, inputs, facts);
    if external_breakpoints == 0 {
        return stamped;
    }
    stamped
        .iter()
        .map(|point| point.restamped(None, Some(external_breakpoints)))
        .collect()
}
