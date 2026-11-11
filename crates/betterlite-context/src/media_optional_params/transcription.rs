use betterlite_models::controls::json_value::JsonValue;

use super::check::drop_unsupported_params;
use super::error::MediaParamsError;
use crate::non_default_params::Params;
use crate::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};
use crate::non_default_params::filters::select_non_default_params;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};
use crate::non_default_params::names::OPENAI_TRANSCRIPTION_PARAMS;

const TRANSCRIPTION_DEFAULTS: &[&str] = &[
    "language",
    "prompt",
    "response_format",
    "temperature",
    "timestamp_granularities",
];

pub struct TranscriptionRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub params: &'request Params,
    pub drop_policy: DropPolicy,
    pub mapper: Option<&'request dyn OptionalParamsMapper>,
}

fn parse_additional_drop_params(params: &Params) -> Result<Vec<String>, MediaParamsError> {
    let names = match params.get("additional_drop_params") {
        None | Some(JsonValue::Null) => Some(Vec::new()),
        Some(JsonValue::Array(items)) => items
            .iter()
            .map(|item| match item {
                JsonValue::Text(name) => Some(name.clone()),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => None,
            })
            .collect(),
        Some(_) => None,
    };
    names.ok_or(MediaParamsError::WrongType(
        "additional_drop_params",
        "a list of strings",
    ))
}

pub fn transcription_optional_params(
    request: &TranscriptionRequest<'_>,
) -> Result<Params, MediaParamsError> {
    let provider = request.custom_llm_provider;
    let drop_policy = request.drop_policy;
    let mut passed_params = request.params.clone();
    passed_params.insert(
        "model".to_owned(),
        JsonValue::Text(request.model.to_owned()),
    );
    let mut non_default_params =
        select_non_default_params(&passed_params, TRANSCRIPTION_DEFAULTS, &[]);
    let optional_params = if matches!(provider, "openai" | "azure") {
        non_default_params
    } else if let Some(mapper) = request.mapper {
        let supported = mapper.supported_openai_params(request.model);
        let drop = drop_policy.litellm_drop_params || drop_policy.drop_params == Some(true);
        drop_unsupported_params(&mut non_default_params, &supported, drop).map_err(|first| {
            MediaParamsError::TranscriptionUnsupported {
                param: first.0,
                provider: provider.to_owned(),
            }
        })?;
        mapper.map_openai_params(
            &non_default_params,
            Params::new(),
            request.model,
            drop_policy.drop_params.unwrap_or(false),
        )?
    } else {
        Params::new()
    };
    let additional_drop_params = parse_additional_drop_params(request.params)?;
    Ok(add_provider_specific_params(
        optional_params,
        &ProviderSpecificRequest {
            passed_params: &passed_params,
            custom_llm_provider: provider,
            openai_params: OPENAI_TRANSCRIPTION_PARAMS,
            additional_drop_params: &additional_drop_params,
        },
    )?)
}
