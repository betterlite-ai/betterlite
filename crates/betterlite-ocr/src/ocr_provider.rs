pub mod handler;
pub(crate) mod resolve;
mod tests;

use crate::azure_ai_ocr::{AZURE_AI_OCR_PATH, AzureAiOcrConfig};
use crate::azure_cohere_ocr::{AZURE_COHERE_PARSE_PATH, AzureAICohereParseConfig};
use crate::azure_doc_intel::AzureDocIntelligenceOcrConfig;
use crate::cohere_ocr::CohereParseConfig;
use crate::mistral_ocr::MistralOcrConfig;
use crate::ocr_config::connection::{OcrCredentialInputs, ResolvedOcrCredentials};
use crate::ocr_config::{OcrProviderConfig as _, PreparedOcrRequest};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::{CallHooks, OcrClient, ocr as run_ocr};
use crate::reducto_ocr::{ReductoParseLegacyConfig, ReductoParseV3Config};
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;
use crate::textract_ocr::analyze::TextractAnalyzeDocumentConfig;
use crate::textract_ocr::detect::TextractDetectTextConfig;
use crate::vertex_deepseek_ocr::VertexAIDeepSeekOCRConfig;
use crate::vertex_ocr::VertexAiOcrConfig;

use resolve::resolve_provider_config;

const AZURE_AI_PROVIDER: &str = "azure_ai";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrConfigKind {
    AwsTextract,
    AwsTextractAnalyze,
    Cohere,
    Mistral,
    AzureAi,
    AzureCohere,
    AzureDocumentIntelligence,
    ReductoLegacy,
    ReductoV3,
    VertexAi,
    VertexDeepSeek,
}

impl OcrConfigKind {
    #[must_use]
    pub const fn provider(self) -> &'static str {
        match self {
            Self::AwsTextract | Self::AwsTextractAnalyze => "aws_textract",
            Self::Cohere => "cohere",
            Self::Mistral => "mistral",
            Self::AzureAi | Self::AzureCohere | Self::AzureDocumentIntelligence => "azure_ai",
            Self::ReductoLegacy | Self::ReductoV3 => "reducto",
            Self::VertexAi | Self::VertexDeepSeek => "vertex_ai",
        }
    }

    #[must_use]
    pub fn get_supported_ocr_params(self, model: &str) -> &'static [&'static str] {
        match self {
            Self::AwsTextract => TextractDetectTextConfig.get_supported_ocr_params(model),
            Self::AwsTextractAnalyze => {
                TextractAnalyzeDocumentConfig.get_supported_ocr_params(model)
            }
            Self::Cohere => CohereParseConfig.get_supported_ocr_params(model),
            Self::Mistral => MistralOcrConfig.get_supported_ocr_params(model),
            Self::AzureAi => AzureAiOcrConfig.get_supported_ocr_params(model),
            Self::AzureCohere => AzureAICohereParseConfig.get_supported_ocr_params(model),
            Self::AzureDocumentIntelligence => {
                AzureDocIntelligenceOcrConfig.get_supported_ocr_params(model)
            }
            Self::ReductoLegacy => {
                ReductoParseLegacyConfig::default().get_supported_ocr_params(model)
            }
            Self::ReductoV3 => ReductoParseV3Config::default().get_supported_ocr_params(model),
            Self::VertexAi => VertexAiOcrConfig.get_supported_ocr_params(model),
            Self::VertexDeepSeek => VertexAIDeepSeekOCRConfig.get_supported_ocr_params(model),
        }
    }

    #[must_use]
    pub fn get_api_key_env_var(self) -> Option<&'static str> {
        match self {
            Self::AwsTextract => TextractDetectTextConfig.get_api_key_env_var(),
            Self::AwsTextractAnalyze => TextractAnalyzeDocumentConfig.get_api_key_env_var(),
            Self::Cohere => CohereParseConfig.get_api_key_env_var(),
            Self::Mistral => MistralOcrConfig.get_api_key_env_var(),
            Self::AzureAi => AzureAiOcrConfig.get_api_key_env_var(),
            Self::AzureCohere => AzureAICohereParseConfig.get_api_key_env_var(),
            Self::AzureDocumentIntelligence => AzureDocIntelligenceOcrConfig.get_api_key_env_var(),
            Self::ReductoLegacy => ReductoParseLegacyConfig::default().get_api_key_env_var(),
            Self::ReductoV3 => ReductoParseV3Config::default().get_api_key_env_var(),
            Self::VertexAi => VertexAiOcrConfig.get_api_key_env_var(),
            Self::VertexDeepSeek => VertexAIDeepSeekOCRConfig.get_api_key_env_var(),
        }
    }

    #[must_use]
    pub fn secret_names(self) -> Vec<&'static str> {
        match self {
            Self::AwsTextract => TextractDetectTextConfig.secret_names(),
            Self::AwsTextractAnalyze => TextractAnalyzeDocumentConfig.secret_names(),
            Self::Cohere => CohereParseConfig.secret_names(),
            Self::Mistral => MistralOcrConfig.secret_names(),
            Self::AzureAi => AzureAiOcrConfig.secret_names(),
            Self::AzureCohere => AzureAICohereParseConfig.secret_names(),
            Self::AzureDocumentIntelligence => AzureDocIntelligenceOcrConfig.secret_names(),
            Self::ReductoLegacy => ReductoParseLegacyConfig::default().secret_names(),
            Self::ReductoV3 => ReductoParseV3Config::default().secret_names(),
            Self::VertexAi => VertexAiOcrConfig.secret_names(),
            Self::VertexDeepSeek => VertexAIDeepSeekOCRConfig.secret_names(),
        }
    }

    #[must_use]
    pub fn get_health_check_document(self) -> OcrDocument {
        match self {
            Self::AwsTextract => TextractDetectTextConfig.get_health_check_document(),
            Self::AwsTextractAnalyze => TextractAnalyzeDocumentConfig.get_health_check_document(),
            Self::Cohere => CohereParseConfig.get_health_check_document(),
            Self::Mistral => MistralOcrConfig.get_health_check_document(),
            Self::AzureAi => AzureAiOcrConfig.get_health_check_document(),
            Self::AzureCohere => AzureAICohereParseConfig.get_health_check_document(),
            Self::AzureDocumentIntelligence => {
                AzureDocIntelligenceOcrConfig.get_health_check_document()
            }
            Self::ReductoLegacy => ReductoParseLegacyConfig::default().get_health_check_document(),
            Self::ReductoV3 => ReductoParseV3Config::default().get_health_check_document(),
            Self::VertexAi => VertexAiOcrConfig.get_health_check_document(),
            Self::VertexDeepSeek => VertexAIDeepSeekOCRConfig.get_health_check_document(),
        }
    }

    #[must_use]
    pub fn resolve_connection_params(self, inputs: OcrCredentialInputs) -> ResolvedOcrCredentials {
        match self {
            Self::AwsTextract => TextractDetectTextConfig.resolve_connection_params(inputs),
            Self::AwsTextractAnalyze => {
                TextractAnalyzeDocumentConfig.resolve_connection_params(inputs)
            }
            Self::Cohere => CohereParseConfig.resolve_connection_params(inputs),
            Self::Mistral => MistralOcrConfig.resolve_connection_params(inputs),
            Self::AzureAi => AzureAiOcrConfig.resolve_connection_params(inputs),
            Self::AzureCohere => AzureAICohereParseConfig.resolve_connection_params(inputs),
            Self::AzureDocumentIntelligence => {
                AzureDocIntelligenceOcrConfig.resolve_connection_params(inputs)
            }
            Self::ReductoLegacy => {
                ReductoParseLegacyConfig::default().resolve_connection_params(inputs)
            }
            Self::ReductoV3 => ReductoParseV3Config::default().resolve_connection_params(inputs),
            Self::VertexAi => VertexAiOcrConfig.resolve_connection_params(inputs),
            Self::VertexDeepSeek => VertexAIDeepSeekOCRConfig.resolve_connection_params(inputs),
        }
    }

    pub fn transform_ocr_response(
        self,
        model: &str,
        body: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        match self {
            Self::AwsTextract => {
                TextractDetectTextConfig.transform_ocr_response(model, body, request_format)
            }
            Self::AwsTextractAnalyze => {
                TextractAnalyzeDocumentConfig.transform_ocr_response(model, body, request_format)
            }
            Self::Cohere => CohereParseConfig.transform_ocr_response(model, body, request_format),
            Self::Mistral => MistralOcrConfig.transform_ocr_response(model, body, request_format),
            Self::AzureAi => AzureAiOcrConfig.transform_ocr_response(model, body, request_format),
            Self::AzureCohere => {
                AzureAICohereParseConfig.transform_ocr_response(model, body, request_format)
            }
            Self::AzureDocumentIntelligence => {
                AzureDocIntelligenceOcrConfig.transform_ocr_response(model, body, request_format)
            }
            Self::ReductoLegacy => ReductoParseLegacyConfig::default().transform_ocr_response(
                model,
                body,
                request_format,
            ),
            Self::ReductoV3 => {
                ReductoParseV3Config::default().transform_ocr_response(model, body, request_format)
            }
            Self::VertexAi => VertexAiOcrConfig.transform_ocr_response(model, body, request_format),
            Self::VertexDeepSeek => {
                VertexAIDeepSeekOCRConfig.transform_ocr_response(model, body, request_format)
            }
        }
    }

    pub async fn ocr(
        self,
        client: &OcrClient,
        request: &PreparedOcrRequest,
        hooks: &dyn CallHooks,
    ) -> Result<OCRResponse, OcrError> {
        match self {
            Self::AwsTextract => run_ocr(&TextractDetectTextConfig, client, request, hooks).await,
            Self::AwsTextractAnalyze => {
                run_ocr(&TextractAnalyzeDocumentConfig, client, request, hooks).await
            }
            Self::Cohere => run_ocr(&CohereParseConfig, client, request, hooks).await,
            Self::Mistral => run_ocr(&MistralOcrConfig, client, request, hooks).await,
            Self::AzureAi => run_ocr(&AzureAiOcrConfig, client, request, hooks).await,
            Self::AzureCohere => run_ocr(&AzureAICohereParseConfig, client, request, hooks).await,
            Self::AzureDocumentIntelligence => {
                run_ocr(&AzureDocIntelligenceOcrConfig, client, request, hooks).await
            }
            Self::ReductoLegacy => {
                run_ocr(&ReductoParseLegacyConfig::default(), client, request, hooks).await
            }
            Self::ReductoV3 => {
                run_ocr(&ReductoParseV3Config::default(), client, request, hooks).await
            }
            Self::VertexAi => run_ocr(&VertexAiOcrConfig, client, request, hooks).await,
            Self::VertexDeepSeek => {
                run_ocr(&VertexAIDeepSeekOCRConfig, client, request, hooks).await
            }
        }
    }
}

pub fn get_api_key_env_var(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Result<Option<&'static str>, OcrError> {
    let (_, kind) = resolve_provider_config(model, custom_llm_provider)?;
    Ok(kind.get_api_key_env_var())
}

pub fn get_health_check_document(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> Result<OcrDocument, OcrError> {
    resolve_provider_config(model, custom_llm_provider)
        .map(|(_, kind)| kind.get_health_check_document())
}

pub fn passthrough_response(
    model: &str,
    endpoint: &str,
    body: &[u8],
) -> Result<Option<OCRResponse>, OcrError> {
    let (resolved_model, kind) = resolve_provider_config(model, Some(AZURE_AI_PROVIDER))?;
    if !azure_ocr_route(kind, &resolved_model, endpoint)? {
        return Ok(None);
    }
    let response = kind.transform_ocr_response(&resolved_model, body, RequestFormat::Litellm)?;
    Ok(Some(response))
}

pub(crate) fn azure_ocr_route(
    kind: OcrConfigKind,
    model: &str,
    endpoint: &str,
) -> Result<bool, OcrError> {
    let segments: Vec<&str> = endpoint
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    match kind {
        OcrConfigKind::AzureAi => Ok(segments.as_slice() == AZURE_AI_OCR_PATH.as_slice()),
        OcrConfigKind::AzureCohere => Ok(segments.as_slice() == AZURE_COHERE_PARSE_PATH.as_slice()),
        OcrConfigKind::AzureDocumentIntelligence => {
            Ok(segments.as_slice()
                == AzureDocIntelligenceOcrConfig::analyze_path(model)?.as_slice())
        }
        other @ (OcrConfigKind::AwsTextract
        | OcrConfigKind::AwsTextractAnalyze
        | OcrConfigKind::Cohere
        | OcrConfigKind::Mistral
        | OcrConfigKind::ReductoLegacy
        | OcrConfigKind::ReductoV3
        | OcrConfigKind::VertexAi
        | OcrConfigKind::VertexDeepSeek) => {
            Err(OcrError::InvalidProvider(other.provider().to_owned()))
        }
    }
}
