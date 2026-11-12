pub mod bedrock;
mod branches;
pub mod error;
pub mod request;
mod tests;

use betterlite_models::controls::json_value::JsonValue;

use crate::non_default_params::Params;
use crate::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};
use branches::{BranchOutcome, branch_params, pre_process};
use error::EmbeddingParamsError;
use request::EmbeddingRequest;

const EMBEDDING_OPENAI_PARAMS: [&str; 6] = [
    "dimensions",
    "encoding_format",
    "user",
    "model",
    "custom_llm_provider",
    "input",
];

pub fn embedding_optional_params(
    request: &EmbeddingRequest<'_>,
) -> Result<Params, EmbeddingParamsError> {
    let (passed_params, mut non_default_params) = pre_process(request)?;
    let dropped =
        request.drop_policy.litellm_drop_params || request.drop_policy.drop_params == Some(true);
    match branch_params(request, &mut non_default_params, &passed_params, dropped)? {
        BranchOutcome::Final(final_params) => Ok(final_params),
        BranchOutcome::Mapped(optional_params) => {
            finalize(request, optional_params, &passed_params)
        }
    }
}

fn finalize(
    request: &EmbeddingRequest<'_>,
    optional_params: Params,
    passed_params: &Params,
) -> Result<Params, EmbeddingParamsError> {
    let mut final_params = add_provider_specific_params(
        optional_params,
        &ProviderSpecificRequest {
            passed_params,
            custom_llm_provider: request.custom_llm_provider,
            openai_params: &EMBEDDING_OPENAI_PARAMS,
            // LiteLLM reads additional_drop_params from **kwargs here, where a named argument never lands
            additional_drop_params: &[],
        },
    )?;
    remove_empty_extra_body(&mut final_params);
    Ok(final_params)
}

fn remove_empty_extra_body(params: &mut Params) {
    let empty = match params.get("extra_body") {
        Some(JsonValue::Object(fields)) => fields.is_empty(),
        Some(JsonValue::Array(items)) => items.is_empty(),
        Some(JsonValue::Text(text)) => text.is_empty(),
        Some(JsonValue::Null) => true,
        Some(JsonValue::Bool(_) | JsonValue::Number(_)) | None => false,
    };
    if empty {
        params.remove("extra_body");
    }
}
