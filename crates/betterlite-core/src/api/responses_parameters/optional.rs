use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::apply_openai_param_overrides;
use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesParametersError;
use super::provider::{DropPolicy, ResponsesParamProvider};
use super::stream_options::normalize_stream_options;

pub struct SupportedParamsRequest<'request> {
    pub provider: &'request str,
    pub model: &'request str,
    pub supported: Option<&'request [String]>,
    pub params: &'request Params,
    pub allowed: &'request [String],
    pub drop_params: bool,
}

pub fn check_supported_params(
    request: &SupportedParamsRequest<'_>,
) -> Result<(), ResponsesParametersError> {
    let Some(supported) = request.supported else {
        return Ok(());
    };
    let unsupported: Vec<&String> = request
        .params
        .keys()
        .filter(|key| {
            !supported.iter().any(|name| name == *key)
                && !request.allowed.iter().any(|name| name == *key)
        })
        .collect();
    if unsupported.is_empty() || request.drop_params {
        return Ok(());
    }
    Err(ResponsesParametersError::UnsupportedParams {
        provider: request.provider.to_owned(),
        model: request.model.to_owned(),
        params: unsupported
            .iter()
            .map(|name| (*name).as_str())
            .collect::<Vec<&str>>()
            .join(", "),
    })
}

pub fn optional_params_responses<P>(
    provider: &P,
    model: &str,
    params: &mut Params,
    allowed_openai_params: &[String],
    policy: DropPolicy,
) -> Result<Params, P::Error>
where
    P: ResponsesParamProvider,
{
    let supported = provider.supported_params(model)?;
    let drop_params = policy.global || policy.call == Some(true);
    check_supported_params(&SupportedParamsRequest {
        provider: provider.name(),
        model,
        supported: supported.as_deref(),
        params,
        allowed: allowed_openai_params,
        drop_params,
    })
    .map_err(P::Error::from)?;
    let mapped = provider.map_params(params, model, drop_params)?;
    let mut normalized = Params::new();
    let stream_options = normalize_stream_options(mapped.get("stream_options"));
    for (key, value) in mapped {
        if key != "stream_options" {
            normalized.insert(key, value);
        }
    }
    if let Some(Some(include_obfuscation)) =
        stream_options.and_then(|options| options.include_obfuscation)
    {
        let fields = IndexMap::from([(
            "include_obfuscation".to_owned(),
            JsonValue::Bool(include_obfuscation),
        )]);
        normalized.insert("stream_options".to_owned(), JsonValue::Object(fields));
    }
    Ok(apply_openai_param_overrides(
        normalized,
        params,
        allowed_openai_params,
    ))
}
