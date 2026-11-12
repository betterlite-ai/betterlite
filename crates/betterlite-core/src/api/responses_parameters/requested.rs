use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::pre_process::{
    PreProcessRequest, base_pre_process_params,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::metadata_filter_limit;
use indexmap::IndexMap;

use super::error::ResponsesParametersError;
use super::nested::apply_nested_drop_params;
use crate::api::response_id_codecs::response::decode_previous_response_id;

pub const RESPONSES_OPTIONAL_PARAMS: &str = "include instructions max_output_tokens metadata \
    parallel_tool_calls previous_response_id reasoning store background stream temperature text \
    tool_choice tools top_p truncation user service_tier safety_identifier prompt max_tool_calls \
    prompt_cache_key prompt_cache_retention prompt_cache_options stream_options top_logprobs \
    partial_images context_management";

pub fn requested_optional_params(params: &mut Params) -> Result<Params, ResponsesParametersError> {
    let provider = match params.remove("custom_llm_provider") {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::Text(name)) => name,
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => {
            return Err(ResponsesParametersError::NotText {
                field: "custom_llm_provider",
            });
        }
    };
    let special_params = match params.remove("kwargs") {
        None | Some(JsonValue::Null) => Params::new(),
        Some(JsonValue::Object(fields)) => fields.into_iter().collect(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Array(_),
        ) => {
            return Err(ResponsesParametersError::NotAnObject { field: "kwargs" });
        }
    };
    let additional_drop_params = additional_drop_params(params.remove("additional_drop_params"))?;
    let processed = base_pre_process_params(
        &PreProcessRequest {
            passed_params: params,
            special_params: &special_params,
            custom_llm_provider: &provider,
            additional_drop_params: &additional_drop_params,
            provider_config: None,
        },
        RESPONSES_OPTIONAL_PARAMS,
        &["input"],
    );
    let mut non_default =
        apply_nested_drop_params(processed.non_default_params, &additional_drop_params)?;
    decode_previous_id(&mut non_default);
    convert_metadata(&mut non_default)?;
    Ok(non_default)
}

fn additional_drop_params(
    value: Option<JsonValue>,
) -> Result<Vec<String>, ResponsesParametersError> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(items)) => items
            .iter()
            .map(|item| match item {
                JsonValue::Text(name) => Ok(name.clone()),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => Err(ResponsesParametersError::NotText {
                    field: "additional_drop_params",
                }),
            })
            .collect(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Text(_) | JsonValue::Object(_),
        ) => Err(ResponsesParametersError::NotList {
            field: "additional_drop_params",
        }),
    }
}

fn decode_previous_id(params: &mut Params) {
    let Some(JsonValue::Text(previous)) = params.get("previous_response_id") else {
        return;
    };
    let decoded = decode_previous_response_id(previous);
    params.insert("previous_response_id".to_owned(), JsonValue::Text(decoded));
}

fn convert_metadata(params: &mut Params) -> Result<(), ResponsesParametersError> {
    let Some(JsonValue::Object(fields)) = params.get("metadata").cloned() else {
        params.remove("metadata");
        return Ok(());
    };
    let visible = fields
        .into_iter()
        .map(|(key, value)| serde_json::to_value(value).map(|json| (key, json)))
        .collect::<Result<IndexMap<_, _>, _>>()
        .map_err(|source| ResponsesParametersError::Json { source })?;
    let Some(converted) = metadata_filter_limit(Some(&visible)) else {
        params.remove("metadata");
        return Ok(());
    };
    let object = converted
        .into_iter()
        .map(|(key, value)| (key, JsonValue::Text(value)))
        .collect();
    params.insert("metadata".to_owned(), JsonValue::Object(object));
    Ok(())
}
