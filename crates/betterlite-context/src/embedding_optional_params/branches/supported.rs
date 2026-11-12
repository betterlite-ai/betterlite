use crate::embedding_optional_params::error::EmbeddingParamsError;
use crate::embedding_optional_params::request::EmbeddingRequest;
use crate::non_default_params::Params;

pub(super) fn reject_unsupported(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    supported: Option<&[String]>,
    dropped: bool,
) -> Result<(), EmbeddingParamsError> {
    let Some(param) = unsupported_param(non_default_params, supported, dropped) else {
        return Ok(());
    };
    Err(rejection(request, param))
}

pub(super) fn unsupported_param<'params>(
    non_default_params: &'params Params,
    supported: Option<&[String]>,
    dropped: bool,
) -> Option<&'params str> {
    if dropped {
        return None;
    }
    let known = supported?;
    non_default_params
        .keys()
        .find(|key| !known.contains(key))
        .map(String::as_str)
}

fn rejection(request: &EmbeddingRequest<'_>, param: &str) -> EmbeddingParamsError {
    EmbeddingParamsError::Unsupported {
        provider: request.custom_llm_provider.to_owned(),
        model: request.model.to_owned(),
        param: param.to_owned(),
    }
}
