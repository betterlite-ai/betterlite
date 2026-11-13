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

macro_rules! with_config {
    ($kind:expr, $config:ident => $body:expr) => {
        match $kind {
            OcrConfigKind::AwsTextract => {
                let $config = TextractDetectTextConfig;
                $body
            }
            OcrConfigKind::AwsTextractAnalyze => {
                let $config = TextractAnalyzeDocumentConfig;
                $body
            }
            OcrConfigKind::Cohere => {
                let $config = CohereParseConfig;
                $body
            }
            OcrConfigKind::Mistral => {
                let $config = MistralOcrConfig;
                $body
            }
            OcrConfigKind::AzureAi => {
                let $config = AzureAiOcrConfig;
                $body
            }
            OcrConfigKind::AzureCohere => {
                let $config = AzureAICohereParseConfig;
                $body
            }
            OcrConfigKind::AzureDocumentIntelligence => {
                let $config = AzureDocIntelligenceOcrConfig;
                $body
            }
            OcrConfigKind::ReductoLegacy => {
                let $config = ReductoParseLegacyConfig::default();
                $body
            }
            OcrConfigKind::ReductoV3 => {
                let $config = ReductoParseV3Config::default();
                $body
            }
            OcrConfigKind::VertexAi => {
                let $config = VertexAiOcrConfig;
                $body
            }
            OcrConfigKind::VertexDeepSeek => {
                let $config = VertexAIDeepSeekOCRConfig;
                $body
            }
        }
    };
}

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
        with_config!(
            self,
            config => config.get_supported_ocr_params(model)
        )
    }

    #[must_use]
    pub fn get_api_key_env_var(self) -> Option<&'static str> {
        with_config!(
            self,
            config => config.get_api_key_env_var()
        )
    }

    #[must_use]
    pub fn secret_names(self) -> Vec<&'static str> {
        with_config!(
            self,
            config => config.secret_names()
        )
    }

    #[must_use]
    pub fn get_health_check_document(self) -> OcrDocument {
        with_config!(
            self,
            config => config.get_health_check_document()
        )
    }

    #[must_use]
    pub fn resolve_connection_params(self, inputs: OcrCredentialInputs) -> ResolvedOcrCredentials {
        with_config!(
            self,
            config => config.resolve_connection_params(inputs)
        )
    }

    pub fn transform_ocr_response(
        self,
        model: &str,
        body: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        with_config!(
            self,
            config => config.transform_ocr_response(model, body, request_format)
        )
    }

    pub async fn ocr(
        self,
        client: &OcrClient,
        request: &PreparedOcrRequest,
        hooks: &dyn CallHooks,
    ) -> Result<OCRResponse, OcrError> {
        with_config!(
            self,
            config => run_ocr(&config, client, request, hooks).await
        )
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
    let (_, kind) = resolve_provider_config(model, custom_llm_provider)?;
    Ok(kind.get_health_check_document())
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
