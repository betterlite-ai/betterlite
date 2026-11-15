use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;

use super::wire::AliceVerdict;
use super::{DEFAULT_BLOCK_MESSAGE, GUARDRAIL_NAME};

pub(super) fn has_selectable_input(inputs: &GuardrailApiInputs) -> bool {
    inputs
        .texts
        .as_ref()
        .is_some_and(|values| !values.is_empty())
        || inputs
            .images
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        || inputs
            .tools
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        || inputs
            .tool_calls
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        || inputs
            .structured_messages
            .as_ref()
            .is_some_and(|values| !values.is_empty())
}

pub(super) fn apply_replacements(
    verdict: &AliceVerdict,
    inputs: &mut GuardrailApiInputs,
) -> Result<(), GuardrailFailure> {
    let replacements = verdict.replacements.as_deref().unwrap_or_default();
    if replacements.is_empty() {
        return Err(blocked(verdict));
    }
    let mut texts = inputs.texts.take().unwrap_or_default();
    for replacement in replacements {
        let candidate = replacement.index.and_then(|raw| usize::try_from(raw).ok());
        let (Some(position), Some(text)) = (candidate, replacement.text.as_deref()) else {
            return Err(blocked(verdict));
        };
        let Some(slot) = texts.get_mut(position) else {
            return Err(blocked(verdict));
        };
        text.clone_into(slot);
    }
    inputs.texts = Some(texts);
    Ok(())
}

pub(super) fn blocked(verdict: &AliceVerdict) -> GuardrailFailure {
    GuardrailFailure::GuardrailRaised {
        guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
        message: verdict
            .message
            .clone()
            .unwrap_or_else(|| DEFAULT_BLOCK_MESSAGE.to_owned()),
        wrap_default_message: false,
        status_code: 400,
        blocked_content: true,
    }
}
