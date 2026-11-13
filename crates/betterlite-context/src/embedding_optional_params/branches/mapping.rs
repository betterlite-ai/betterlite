use betterlite_models::controls::json_value::JsonValue;

use super::BranchOutcome;
use super::supported::reject_unsupported;
use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::embedding_optional_params::request::EmbeddingRequest;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::OptionalParamsMapper;
use crate::non_default_params::names::OPENAI_EMBEDDING_PARAMS;

pub(super) fn config_supported_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    let supported = mapper(request)?.supported_openai_params(request.model);
    let mapped = delegating_params(request, non_default_params, Some(&supported), dropped)?;
    Ok(BranchOutcome::Mapped(mapped))
}

pub(super) fn delegating_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    supported: Option<&[String]>,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    reject_unsupported(request, non_default_params, supported, dropped)?;
    map_through_mapper(request, non_default_params)
}

pub(super) fn provider_config_params(
    request: &EmbeddingRequest<'_>,
    config: &dyn OptionalParamsMapper,
    non_default_params: &Params,
    passed_params: &Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    let supported = config.supported_openai_params(request.model);
    reject_unsupported(request, non_default_params, Some(&supported), dropped)?;
    let mut optional_params = config.map_openai_params(
        non_default_params,
        Params::new(),
        request.model,
        request.drop_policy.drop_params.unwrap_or(false),
    )?;
    restore_provider_only_params(&mut optional_params, &supported, passed_params);
    Ok(optional_params)
}

fn map_through_mapper(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
) -> Result<Params, EmbeddingParamsError> {
    Ok(mapper(request)?.map_openai_params(
        non_default_params,
        Params::new(),
        request.model,
        request.drop_policy.drop_params.unwrap_or(false),
    )?)
}

fn mapper<'request>(
    request: &'request EmbeddingRequest<'_>,
) -> Result<&'request dyn OptionalParamsMapper, EmbeddingParamsError> {
    request
        .mapper
        .ok_or_else(|| EmbeddingParamsError::MissingMapper {
            provider: request.custom_llm_provider.to_owned(),
        })
}

fn restore_provider_only_params(
    optional_params: &mut Params,
    supported: &[String],
    passed_params: &Params,
) {
    for param in supported {
        let is_openai_param = OPENAI_EMBEDDING_PARAMS
            .split_whitespace()
            .any(|known| known == param);
        if is_openai_param || optional_params.contains_key(param) {
            continue;
        }
        if let Some(value) = passed_params.get(param)
            && !matches!(value, JsonValue::Null)
        {
            optional_params.insert(param.clone(), value.clone());
        }
    }
}
