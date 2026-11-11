pub mod arguments;
mod tests;
pub mod wire;

use crate::json::{Json, Object};
use crate::ocr_config::AzureAdTokenProvider;
use crate::ocr_config::OCR_RESPONSE_MAX_BYTES;
use crate::ocr_config::connection::{OcrCredentialInputs, OcrInputSource, OcrTransportConfig};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_provider::OcrConfigKind;
use crate::ocr_provider::resolve::resolve_provider_config;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrDocumentInput {
    Document(OcrDocument),
    Path {
        path: PathBuf,
        mime_type: Option<String>,
    },
    Bytes {
        bytes: Vec<u8>,
        file_name: Option<String>,
        mime_type: Option<String>,
    },
}

impl From<OcrDocument> for OcrDocumentInput {
    fn from(document: OcrDocument) -> Self {
        Self::Document(document)
    }
}

impl From<PathBuf> for OcrDocumentInput {
    fn from(path: PathBuf) -> Self {
        Self::Path {
            path,
            mime_type: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct OcrConnectionInputs {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub extra_headers: Object,
    pub timeout: Option<Duration>,
    pub input_sources: BTreeMap<String, OcrInputSource>,
}

impl OcrConnectionInputs {
    fn header_pairs(&self) -> Result<Vec<(String, String)>, OcrError> {
        self.extra_headers
            .iter()
            .map(|(name, value)| match value {
                Json::Text(text) => Ok((name.clone(), text.clone())),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(OcrError::RequestField {
                        path: format!("extra_headers.{name}"),
                    })
                }
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct LiteLLMOcrRequest<D = OcrDocumentInput> {
    pub model: String,
    pub document: D,
    pub credentials: OcrCredentialInputs,
    pub transport: OcrTransportConfig,
    pub custom_llm_provider: Option<String>,
    pub optional_params: Object,
    pub input_sources: BTreeMap<String, OcrInputSource>,
    pub azure_ad_token_provider: Option<Arc<dyn AzureAdTokenProvider>>,
    pub config: OcrConfigKind,
}

impl LiteLLMOcrRequest {
    pub fn new(
        model: &str,
        document: OcrDocumentInput,
        custom_llm_provider: Option<&str>,
        mut optional_params: Object,
    ) -> Result<Self, OcrError> {
        let (resolved_model, config) = resolve_provider_config(model, custom_llm_provider)?;
        let invalid = || OcrError::RequestField {
            path: "max_response_bytes".to_owned(),
        };
        let max_response_bytes = match optional_params.shift_remove("max_response_bytes") {
            None => OCR_RESPONSE_MAX_BYTES,
            Some(Json::Number(number)) => number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0 && *value <= OCR_RESPONSE_MAX_BYTES)
                .ok_or_else(invalid)?,
            Some(_) => return Err(invalid()),
        };
        let transport = OcrTransportConfig {
            extra_headers: Vec::new(),
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes,
        };
        Ok(Self {
            model: resolved_model,
            document,
            credentials: OcrCredentialInputs::default(),
            transport,
            custom_llm_provider: custom_llm_provider.map(str::to_owned),
            optional_params,
            input_sources: BTreeMap::new(),
            azure_ad_token_provider: None,
            config,
        })
    }

    pub fn from_inputs(
        model: &str,
        document: OcrDocumentInput,
        custom_llm_provider: Option<&str>,
        optional_params: Object,
        connection: OcrConnectionInputs,
    ) -> Result<Self, OcrError> {
        let mut request = Self::new(model, document, custom_llm_provider, optional_params)?;
        request.apply_connection(connection)?;
        Ok(request)
    }

    fn apply_connection(&mut self, connection: OcrConnectionInputs) -> Result<(), OcrError> {
        let extra_headers = connection.header_pairs()?;
        let transport = mem::take(&mut self.transport);
        self.transport = transport.with_overrides(
            extra_headers,
            connection
                .input_sources
                .get("extra_headers")
                .copied()
                .unwrap_or_default(),
            connection.timeout,
        );
        self.credentials = OcrCredentialInputs::new(
            connection.api_key,
            connection
                .input_sources
                .get("api_key")
                .copied()
                .unwrap_or_default(),
            connection.api_base,
            connection
                .input_sources
                .get("api_base")
                .copied()
                .unwrap_or_default(),
        );
        self.input_sources = connection.input_sources;
        Ok(())
    }

    pub fn map_document<T, F>(self, map: F) -> Result<LiteLLMOcrRequest<T>, OcrError>
    where
        F: FnOnce(OcrDocumentInput) -> Result<T, OcrError>,
    {
        let document = map(self.document)?;
        Ok(LiteLLMOcrRequest {
            model: self.model,
            document,
            credentials: self.credentials,
            transport: self.transport,
            custom_llm_provider: self.custom_llm_provider,
            optional_params: self.optional_params,
            input_sources: self.input_sources,
            azure_ad_token_provider: self.azure_ad_token_provider,
            config: self.config,
        })
    }
}
