use betterlite_openai::decisions::call::{DecisionsCall, PrepareCall, prepare_call};

use super::super::{DecisionsArgs, DecisionsEntryError, DecisionsPrepare};
use super::request_fields::request_fields;

mod tests;

const STATE_KEY: &str = "state";

// _prepare_decisions_call: the fields come from _request_fields, and state never travels on as a litellm param
pub(crate) fn prepare_decisions_call<'deps>(
    prepare: DecisionsPrepare<'deps>,
    args: DecisionsArgs,
) -> Result<DecisionsCall<'deps>, DecisionsEntryError> {
    let DecisionsArgs {
        model,
        input,
        questions,
        safety_identifier,
        api_key,
        api_base,
        timeout,
        custom_llm_provider,
        extra_headers,
        mut kwargs,
    } = args;
    let fields =
        request_fields(input, questions, &kwargs).map_err(|error| DecisionsEntryError::Fields {
            message: error.to_string(),
        })?;
    kwargs.shift_remove(STATE_KEY);
    prepare_call(PrepareCall {
        model: &model,
        fields,
        safety_identifier: safety_identifier.as_deref(),
        api_key: api_key.as_ref(),
        api_base: api_base.as_deref(),
        timeout,
        custom_llm_provider: custom_llm_provider.as_deref(),
        extra_headers: extra_headers.as_ref(),
        kwargs,
        global_drop_params: prepare.global_drop_params,
        globals: prepare.globals,
        azure_ai: prepare.azure_ai,
        resolution: prepare.resolution,
        logging_obj: prepare.logging,
    })
    .map_err(DecisionsEntryError::from)
}
