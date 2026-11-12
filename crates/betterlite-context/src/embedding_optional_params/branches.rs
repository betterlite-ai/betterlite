use betterlite_models::controls::json_value::JsonValue;

use super::bedrock::{BedrockEmbeddingFamily, bedrock_embedding_family};
use super::error::EmbeddingParamsError;
use super::request::EmbeddingRequest;
use crate::non_default_params::Params;
use crate::non_default_params::names::OPENAI_EMBEDDING_PARAMS;
use crate::non_default_params::pre_process::{PreProcessRequest, base_pre_process_params};
use mapping::{config_supported_params, delegating_params, provider_config_params};
use plain::plain_params;
use supported::reject_unsupported;

mod mapping;
mod plain;
mod supported;

const NO_SUPPORTED_PARAMS: [String; 0] = [];

pub(super) enum BranchOutcome {
    Mapped(Params),
    Final(Params),
}

pub(super) fn pre_process(
    request: &EmbeddingRequest<'_>,
) -> Result<(Params, Params), EmbeddingParamsError> {
    let processed = base_pre_process_params(
        &PreProcessRequest {
            passed_params: request.passed_params,
            special_params: request.special_params,
            custom_llm_provider: request.custom_llm_provider,
            additional_drop_params: request.additional_drop_params,
            provider_config: None,
        },
        OPENAI_EMBEDDING_PARAMS,
        &["input"],
    );
    let passed_params = processed.passed_params;
    if let Some(value) = passed_params.get("extra_body")
        && !matches!(value, JsonValue::Null | JsonValue::Object(_))
    {
        return Err(EmbeddingParamsError::WrongType("extra_body", "an object"));
    }
    Ok((passed_params, processed.non_default_params))
}

pub(super) fn branch_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &mut Params,
    passed_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    if let Some(config) = request.provider_config {
        let mapped =
            provider_config_params(request, config, non_default_params, passed_params, dropped)?;
        return Ok(BranchOutcome::Mapped(mapped));
    }
    match request.custom_llm_provider {
        "openai" | "ollama" => Ok(BranchOutcome::Mapped(mutating_params(
            request,
            non_default_params,
            dropped,
        )?)),
        "vertex_ai" | "gemini" => {
            let encoding_format = non_default_params.get("encoding_format");
            if matches!(encoding_format, Some(JsonValue::Text(format)) if format.as_str() == "float")
            {
                non_default_params.remove("encoding_format");
            }
            let mapped =
                delegating_params(request, non_default_params, request.supported, dropped)?;
            Ok(BranchOutcome::Mapped(mapped))
        }
        "voyage" | "infinity" => {
            let mut final_params =
                delegating_params(request, non_default_params, request.supported, dropped)?;
            final_params.extend(request.special_params.clone());
            Ok(BranchOutcome::Final(final_params))
        }
        "bedrock" => bedrock_params(request, non_default_params, dropped),
        "lm_studio" => config_supported_params(request, non_default_params, dropped),
        provider => plain_params(request, provider, non_default_params, dropped),
    }
}

fn mutating_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &mut Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    if request.custom_llm_provider == "ollama" {
        let mut optional_params = Params::new();
        if let Some(dimensions) = non_default_params.remove("dimensions") {
            optional_params.insert("dimensions".to_owned(), dimensions);
        }
        let supported = ["dimensions".to_owned()];
        reject_unsupported(request, non_default_params, Some(&supported), dropped)?;
        return Ok(optional_params);
    }
    let dimensions_allowed = request
        .allowed_openai_params
        .iter()
        .any(|name| name == "dimensions");
    if !request.model.contains("text-embedding-3")
        && non_default_params.contains_key("dimensions")
        && !dimensions_allowed
    {
        if !dropped {
            return Err(EmbeddingParamsError::DimensionsUnsupported {
                model: request.model.to_owned(),
            });
        }
        non_default_params.remove("dimensions");
    }
    Ok(non_default_params.clone())
}

fn bedrock_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    if bedrock_embedding_family(request.model) == BedrockEmbeddingFamily::Unmapped {
        reject_unsupported(
            request,
            non_default_params,
            Some(&NO_SUPPORTED_PARAMS),
            dropped,
        )?;
        return Ok(BranchOutcome::Final(request.special_params.clone()));
    }
    config_supported_params(request, non_default_params, dropped)
}
