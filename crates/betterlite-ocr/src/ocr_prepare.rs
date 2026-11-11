mod tests;

use crate::azure_ai_ocr::AZURE_AI_API_BASE_ENV;
use crate::ocr_config::connection::{
    OcrConnection, OcrCredentialInputs, OcrInputSource, OcrSourced, ResolvedOcrCredentials,
};
use crate::ocr_config::{OCR_INLINE_MAX_BYTES, PreparedOcrRequest, Secrets};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_http::OcrClient;
use crate::ocr_input::{LiteLLMOcrRequest, OcrDocumentInput};
use crate::ocr_provider::OcrConfigKind;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

const MISTRAL_AZURE_API_KEY_ENV: &str = "MISTRAL_AZURE_API_KEY";
const MISTRAL_AZURE_API_BASE_ENV: &str = "MISTRAL_AZURE_API_BASE";

pub fn prepare_document(input: OcrDocumentInput) -> Result<OcrDocument, OcrError> {
    match input {
        OcrDocumentInput::Document(document) => Ok(document),
        OcrDocumentInput::Path { path, mime_type } => {
            read_path_document(&path, mime_type.as_deref())
        }
        OcrDocumentInput::Bytes {
            bytes,
            file_name,
            mime_type,
        } => encode_file_document(&bytes, file_name.as_deref(), mime_type.as_deref()),
    }
}

pub fn read_path_document(path: &Path, mime_type: Option<&str>) -> Result<OcrDocument, OcrError> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(OCR_INLINE_MAX_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    File::open(path)
        .and_then(|file| file.take(limit).read_to_end(&mut bytes))
        .map_err(|source| OcrError::FileRead {
            path: path.to_owned(),
            source,
        })?;
    let name = path.file_name().and_then(|name| name.to_str());
    encode_file_document(&bytes, name, mime_type)
}

pub fn encode_file_document(
    bytes: &[u8],
    file_name: Option<&str>,
    mime_type: Option<&str>,
) -> Result<OcrDocument, OcrError> {
    if bytes.is_empty() {
        return Err(OcrError::EmptyFile);
    }
    if bytes.len() > OCR_INLINE_MAX_BYTES {
        return Err(OcrError::InlineDocumentTooLarge);
    }
    if let Some(value) = mime_type
        && !valid_mime_type(value)
    {
        return Err(OcrError::InvalidMimeType(value.to_owned()));
    }
    let resolved_mime = mime_type
        .map(str::to_owned)
        .or_else(|| file_name.map(|name| mime_type_for_name(name).to_owned()))
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    let source = format!("data:{resolved_mime};base64,{}", STANDARD.encode(bytes));
    if resolved_mime.starts_with("image/") {
        return Ok(OcrDocument::ImageUrl {
            image_url: source,
            extra: BTreeMap::new(),
        });
    }
    Ok(OcrDocument::DocumentUrl {
        document_url: source,
        extra: BTreeMap::new(),
    })
}

fn valid_mime_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    !kind.is_empty()
        && !subtype.is_empty()
        && kind.chars().chain(subtype.chars()).all(|character| {
            character.is_alphanumeric() || matches!(character, '.' | '+' | '-' | '_')
        })
}

#[must_use]
pub fn mime_type_for_name(name: &str) -> &'static str {
    if let Some(mime) = mime_guess::from_path(name).first_raw() {
        return mime;
    }
    "application/octet-stream"
}

const fn connection_env_names(kind: OcrConfigKind) -> (Option<&'static str>, Option<&'static str>) {
    match kind {
        OcrConfigKind::Mistral => (
            Some(MISTRAL_AZURE_API_KEY_ENV),
            Some(MISTRAL_AZURE_API_BASE_ENV),
        ),
        OcrConfigKind::AzureAi
        | OcrConfigKind::AzureCohere
        | OcrConfigKind::AzureDocumentIntelligence => (None, Some(AZURE_AI_API_BASE_ENV)),
        OcrConfigKind::AwsTextract
        | OcrConfigKind::AwsTextractAnalyze
        | OcrConfigKind::Cohere
        | OcrConfigKind::ReductoLegacy
        | OcrConfigKind::ReductoV3
        | OcrConfigKind::VertexAi
        | OcrConfigKind::VertexDeepSeek => (None, None),
    }
}

fn resolve_credentials(
    request: &LiteLLMOcrRequest<OcrDocument>,
    secrets: &Secrets,
) -> ResolvedOcrCredentials {
    let credentials = request.credentials.clone();
    let (preferred_api_key_env, api_base_env) = connection_env_names(request.config);
    let environment = |name: &str| {
        secrets
            .get(name)
            .filter(|value| !value.expose_secret().is_empty())
            .cloned()
    };
    let dynamic_api_key = credentials.dynamic_api_key.or_else(|| {
        credentials.api_key.clone().or_else(|| {
            preferred_api_key_env
                .into_iter()
                .chain(request.config.get_api_key_env_var())
                .find_map(environment)
                .map(|value| OcrSourced {
                    value,
                    source: OcrInputSource::Environment,
                })
        })
    });
    let dynamic_api_base = credentials.dynamic_api_base.or_else(|| {
        credentials.api_base.clone().or_else(|| {
            api_base_env.and_then(environment).map(|secret| OcrSourced {
                value: secret.expose_secret().to_owned(),
                source: OcrInputSource::Environment,
            })
        })
    });
    request
        .config
        .resolve_connection_params(OcrCredentialInputs {
            dynamic_api_key,
            dynamic_api_base,
            ..credentials
        })
}

#[must_use]
pub fn prepare_request(
    request: LiteLLMOcrRequest<OcrDocument>,
    caller_document: bool,
    client: &OcrClient,
    secrets: Secrets,
) -> PreparedOcrRequest {
    let resolved = resolve_credentials(&request, &secrets);
    let LiteLLMOcrRequest {
        model,
        document,
        transport,
        optional_params,
        input_sources,
        azure_ad_token_provider,
        ..
    } = request;
    PreparedOcrRequest {
        model,
        document,
        connection: OcrConnection::new(resolved, transport, client.settings.clone(), secrets),
        caller_document,
        optional_params,
        input_sources,
        azure_ad_token_provider,
    }
}
