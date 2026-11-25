use betterlite_openai::decisions::call::fields::{RequestFields, SystemOneRequestFields};
use betterlite_openai::decisions::call::{
    DecisionsCall, PrepareCall, prepare_call, reject_other_format,
};

use super::super::{SystemOneArgs, SystemOneEntryError};
use crate::decisions::DecisionsPrepare;
use crate::decisions::decisions_entrypoint::request_fields::systemone_questions;

mod tests;

const INPUT_KEY: &str = "input";

// _prepare_systemone_call: the input keyword belongs to litellm.decisions, so it is rejected before anything else
pub(crate) fn prepare_systemone_call<'deps>(
    prepare: DecisionsPrepare<'deps>,
    args: SystemOneArgs,
) -> Result<DecisionsCall<'deps>, SystemOneEntryError> {
    let SystemOneArgs {
        model,
        state,
        questions,
        safety_identifier,
        api_key,
        api_base,
        timeout,
        custom_llm_provider,
        extra_headers,
        kwargs,
    } = args;
    reject_other_format(&model, &kwargs, INPUT_KEY, "decisions")?;
    let questions =
        systemone_questions(questions).map_err(|error| SystemOneEntryError::Fields {
            message: error.to_string(),
        })?;
    prepare_call(PrepareCall {
        model: &model,
        fields: RequestFields::SystemOne(SystemOneRequestFields { state, questions }),
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
    .map_err(SystemOneEntryError::from)
}
