mod tests;

use crate::context::PolicyMatchContext;
use crate::values::PolicyCondition;

#[must_use]
pub fn evaluate_condition(
    condition: Option<&PolicyCondition>,
    context: &PolicyMatchContext,
) -> bool {
    let Some(policy_condition) = condition else {
        return true;
    };
    let Some(model_condition) = policy_condition.model.as_ref() else {
        return true;
    };
    let matched = context.model.as_deref().is_some_and(|model| {
        model_condition
            .patterns()
            .iter()
            .any(|pattern| model_matches(pattern, model))
    });
    if !matched {
        let model = context.model.as_deref().unwrap_or("None");
        let patterns = &policy_condition.model;
        tracing::debug!("Condition failed: model={model} did not match {patterns:?}");
    }
    matched
}

fn model_matches(pattern: &str, model: &str) -> bool {
    if pattern == model {
        return true;
    }
    // the bare compile rejects what Python's re rejects; the wrapper alone would turn x)|(.* into a match-all
    if fancy_regex::Regex::new(pattern).is_err() {
        return false;
    }
    let anchored = format!(r"\A(?:{pattern})\z");
    let Ok(regex) = fancy_regex::Regex::new(&anchored) else {
        return false;
    };
    regex.is_match(model).unwrap_or(false)
}
