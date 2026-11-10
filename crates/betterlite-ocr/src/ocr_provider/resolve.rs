use crate::ocr_error::OcrError;
use crate::ocr_provider::OcrConfigKind;
use crate::textract_ocr::TextractOperation;
use strum::{EnumString, IntoStaticStr};

const DEFAULT_PROVIDER: &str = "mistral";

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
enum Provider {
    Anthropic,
    AwsTextract,
    AzureAi,
    Bedrock,
    Cohere,
    Mistral,
    Openai,
    OpenaiLike,
    Reducto,
    VertexAi,
}

struct CustomProvider<'model> {
    model: &'model str,
    provider: &'model str,
}

pub(crate) fn resolve_provider_config(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Result<(String, OcrConfigKind), OcrError> {
    let resolved = explicit_provider(model, custom_llm_provider).unwrap_or(CustomProvider {
        model,
        provider: DEFAULT_PROVIDER,
    });
    let Ok(provider) = resolved.provider.parse::<Provider>() else {
        return Err(OcrError::InvalidProvider(resolved.provider.to_owned()));
    };
    let kind = kind_for(provider, resolved.model, resolved.provider)?;
    Ok((resolved.model.to_owned(), kind))
}

fn explicit_provider<'model>(
    model: &'model str,
    custom_llm_provider: Option<&'model str>,
) -> Option<CustomProvider<'model>> {
    if let Some(custom) = custom_llm_provider.filter(|provider| !provider.is_empty()) {
        return Some(CustomProvider {
            model: strip_provider_prefix(model, custom),
            provider: custom,
        });
    }
    let (prefix, rest) = model.split_once('/')?;
    if prefix.is_empty() || rest.is_empty() {
        return None;
    }
    Some(CustomProvider {
        model: rest,
        provider: prefix,
    })
}

fn strip_provider_prefix<'model>(model: &'model str, provider: &str) -> &'model str {
    model
        .strip_prefix(provider)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(model)
}

fn kind_for(
    provider: Provider,
    model: &str,
    provider_name: &str,
) -> Result<OcrConfigKind, OcrError> {
    Ok(match provider {
        Provider::AwsTextract => textract_kind(model)?,
        Provider::Cohere => OcrConfigKind::Cohere,
        Provider::Mistral => OcrConfigKind::Mistral,
        Provider::AzureAi => azure_kind(model),
        Provider::Reducto if model.eq_ignore_ascii_case("parse-legacy") => {
            OcrConfigKind::ReductoLegacy
        }
        Provider::Reducto => OcrConfigKind::ReductoV3,
        Provider::VertexAi if model.to_ascii_lowercase().contains("deepseek") => {
            OcrConfigKind::VertexDeepSeek
        }
        Provider::VertexAi => OcrConfigKind::VertexAi,
        Provider::Anthropic | Provider::Bedrock | Provider::Openai | Provider::OpenaiLike => {
            return Err(OcrError::InvalidProvider(provider_name.to_owned()));
        }
    })
}

fn textract_kind(model: &str) -> Result<OcrConfigKind, OcrError> {
    Ok(match TextractOperation::from_model(model)? {
        TextractOperation::DetectDocumentText => OcrConfigKind::AwsTextract,
        TextractOperation::AnalyzeDocument => OcrConfigKind::AwsTextractAnalyze,
    })
}

fn azure_kind(model: &str) -> OcrConfigKind {
    let lowered = model.to_ascii_lowercase();
    if lowered.contains("doc-intelligence") || lowered.contains("documentintelligence") {
        return OcrConfigKind::AzureDocumentIntelligence;
    }
    if lowered.contains("cohere") && lowered.contains("parse") {
        return OcrConfigKind::AzureCohere;
    }
    OcrConfigKind::AzureAi
}
