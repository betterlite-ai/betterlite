use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::response_params::OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS;
use betterlite_openai::response_params::params::supported_openai_params;
use serde_json::Number;

use crate::api::responses_provider_contract::records::Params;

use super::error::ManusResponsesError;

const MAX_OUTPUT_TOKENS_FIELD: &str = "max_output_tokens";

pub(super) fn supported_params() -> Vec<String> {
    supported_openai_params()
        .into_iter()
        .map(String::from)
        .collect()
}

pub(super) fn map_params(
    optional_params: &ResponsesApiRequestParams,
) -> Result<Params, ManusResponsesError> {
    let value = serde_json::to_value(optional_params)?;
    let Json::Object(mut fields) = serde_json::from_value::<Json>(value)? else {
        return Err(ManusResponsesError::ParamsShape);
    };
    fields.shift_remove("input");
    fields.shift_remove("model");
    if let Some(Json::Number(number)) = fields.get(MAX_OUTPUT_TOKENS_FIELD)
        && let Some(tokens) = number.as_i64()
    {
        let minimum = i64::from(OPENAI_RESPONSES_API_MIN_MAX_OUTPUT_TOKENS);
        if tokens < minimum {
            fields.insert(
                String::from(MAX_OUTPUT_TOKENS_FIELD),
                Json::Number(Number::from(minimum)),
            );
        }
    }
    Ok(fields.into_iter().collect())
}
