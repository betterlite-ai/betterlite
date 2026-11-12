use betterlite_models::controls::json_value::JsonValue;

use super::bedrock::{BedrockEmbeddingFamily, bedrock_embedding_family};
use super::error::EmbeddingParamsError;
use super::request::EmbeddingRequest;
use crate::non_default_params::Params;
use crate::non_default_params::mapper::OptionalParamsMapper;
use crate::non_default_params::names::{OPENAI_EMBEDDING_PARAMS, is_openai_compatible_provider};
use crate::non_default_params::pre_process::{PreProcessRequest, base_pre_process_params};

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
        "openai" => Ok(BranchOutcome::Mapped(openai_params(
            request,
            non_default_params,
            dropped,
        )?)),
        "vertex_ai" | "gemini" => Ok(BranchOutcome::Mapped(vertex_params(
            request,
            non_default_params,
            dropped,
        )?)),
        "bedrock" => bedrock_params(request, non_default_params, dropped),
        "lm_studio" => Ok(BranchOutcome::Mapped(config_supported_params(
            request,
            non_default_params,
            dropped,
        )?)),
        "voyage" | "infinity" => merged_params(request, non_default_params, dropped),
        "ollama" => Ok(BranchOutcome::Mapped(ollama_params(
            request,
            non_default_params,
            dropped,
        )?)),
        provider => plain_params(request, provider, non_default_params, dropped),
    }
}

fn provider_config_params(
    request: &EmbeddingRequest<'_>,
    config: &dyn OptionalParamsMapper,
    non_default_params: &Params,
    passed_params: &Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    let supported = config.supported_openai_params(request.model);
    check_supported(request, non_default_params, Some(&supported), dropped)?;
    let mut optional_params = map_params(config, request, non_default_params)?;
    restore_provider_only_params(&mut optional_params, &supported, passed_params);
    Ok(optional_params)
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

fn openai_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &mut Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
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

fn vertex_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &mut Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    let encoding_format = non_default_params.get("encoding_format");
    if matches!(encoding_format, Some(JsonValue::Text(format)) if format.as_str() == "float") {
        non_default_params.remove("encoding_format");
    }
    delegate_params(request, non_default_params, dropped)
}

fn bedrock_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    match bedrock_embedding_family(request.model) {
        BedrockEmbeddingFamily::Unmapped => {
            check_supported(
                request,
                non_default_params,
                Some(&NO_SUPPORTED_PARAMS),
                dropped,
            )?;
            Ok(BranchOutcome::Final(request.special_params.clone()))
        }
        BedrockEmbeddingFamily::TitanTextV1
        | BedrockEmbeddingFamily::TitanImageV1
        | BedrockEmbeddingFamily::TitanTextV2
        | BedrockEmbeddingFamily::CohereEmbed
        | BedrockEmbeddingFamily::TwelveLabsMarengo
        | BedrockEmbeddingFamily::Nova => Ok(BranchOutcome::Mapped(config_supported_params(
            request,
            non_default_params,
            dropped,
        )?)),
    }
}

fn merged_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    let mut final_params = delegate_params(request, non_default_params, dropped)?;
    final_params.extend(request.special_params.clone());
    Ok(BranchOutcome::Final(final_params))
}

fn ollama_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &mut Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    let mut optional_params = Params::new();
    if let Some(dimensions) = non_default_params.remove("dimensions") {
        optional_params.insert("dimensions".to_owned(), dimensions);
    }
    let supported = ["dimensions".to_owned()];
    check_supported(request, non_default_params, Some(&supported), dropped)?;
    Ok(optional_params)
}

fn plain_params(
    request: &EmbeddingRequest<'_>,
    provider: &str,
    non_default_params: &Params,
    dropped: bool,
) -> Result<BranchOutcome, EmbeddingParamsError> {
    if is_delegating_provider(provider) {
        let mapped = delegate_params(request, non_default_params, dropped)?;
        return Ok(BranchOutcome::Mapped(mapped));
    }
    if provider == "azure" || is_openai_compatible_provider(provider) {
        return Ok(BranchOutcome::Mapped(non_default_params.clone()));
    }
    check_supported(
        request,
        non_default_params,
        Some(&NO_SUPPORTED_PARAMS),
        dropped,
    )?;
    Ok(BranchOutcome::Mapped(Params::new()))
}

fn is_delegating_provider(provider: &str) -> bool {
    matches!(
        provider,
        "triton"
            | "databricks"
            | "nvidia_nim"
            | "mistral"
            | "jina_ai"
            | "sap"
            | "fireworks_ai"
            | "sambanova"
            | "ovhcloud"
    )
}

fn config_supported_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    let mapper = request
        .mapper
        .ok_or_else(|| EmbeddingParamsError::MissingMapper {
            provider: request.custom_llm_provider.to_owned(),
        })?;
    let supported = mapper.supported_openai_params(request.model);
    check_supported(request, non_default_params, Some(&supported), dropped)?;
    map_params(mapper, request, non_default_params)
}

fn delegate_params(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    dropped: bool,
) -> Result<Params, EmbeddingParamsError> {
    check_supported(request, non_default_params, request.supported, dropped)?;
    let mapper = request
        .mapper
        .ok_or_else(|| EmbeddingParamsError::MissingMapper {
            provider: request.custom_llm_provider.to_owned(),
        })?;
    map_params(mapper, request, non_default_params)
}

fn map_params(
    mapper: &dyn OptionalParamsMapper,
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
) -> Result<Params, EmbeddingParamsError> {
    let drop_params = request.drop_policy.drop_params.unwrap_or(false);
    Ok(mapper.map_openai_params(
        non_default_params,
        Params::new(),
        request.model,
        drop_params,
    )?)
}

fn check_supported(
    request: &EmbeddingRequest<'_>,
    non_default_params: &Params,
    supported: Option<&[String]>,
    dropped: bool,
) -> Result<(), EmbeddingParamsError> {
    let Some(known) = supported else {
        return Ok(());
    };
    if dropped || non_default_params.keys().all(|key| known.contains(key)) {
        return Ok(());
    }
    Err(EmbeddingParamsError::Unsupported {
        provider: request.custom_llm_provider.to_owned(),
        model: request.model.to_owned(),
    })
}
