use betterlite_core::search::{SearchOptions, SearchRequest};
use betterlite_cost::search::params::SearchOptionalParams;
use betterlite_models::decisions::Json;
use betterlite_ocr::json::{Json as OcrJson, Object as OcrObject};
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::ocr_input::OcrDocumentInput;
use betterlite_ocr::ocr_input::wire::decode_document;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

mod tests;

const FILE_DOCUMENT_TYPE: &str = "file";
const FILE_MARKER_KEY: &str = "$file";
// python-bridge/src/routes/ocr/document.rs: the three failures a file document can raise before any read
const MISSING_FILE: &str = "document with type='file' must include a 'file' field containing a pathlib.Path, file-like object, or bytes";
const BARE_STRING: &str = "OCR file input does not accept bare str values. Pass bytes, a pathlib.Path, or a file-like object.";
const UNSUPPORTED_FILE: &str =
    "Unsupported file input type. Expected pathlib.Path, bytes, or a file-like object.";

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the timeout of {seconds} seconds is not a valid duration")]
    Timeout { seconds: f64 },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SessionWire {
    // absent means the process environment, the snapshot LiteLLM reads its globals from at import
    pub(crate) environment: Option<BTreeMap<String, String>>,
}

// litellm.search's public arguments, one field per public keyword; options and kwargs stay apart because kwargs override them
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchCall {
    pub(crate) query: Json,
    pub(crate) search_provider: String,
    #[serde(default)]
    pub(crate) max_results: Option<i64>,
    #[serde(default)]
    pub(crate) search_domain_filter: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) max_tokens_per_page: Option<i64>,
    #[serde(default)]
    pub(crate) country: Option<String>,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) timeout_seconds: Option<f64>,
    #[serde(default)]
    pub(crate) extra_headers: Option<Headers>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
}

impl SearchCall {
    pub(crate) fn into_request(self) -> Result<SearchRequest, ConvertError> {
        let timeout = match self.timeout_seconds {
            Some(seconds) => Some(
                Duration::try_from_secs_f64(seconds)
                    .map_err(|_source| ConvertError::Timeout { seconds })?,
            ),
            None => None,
        };
        Ok(SearchRequest {
            query: self.query,
            search_provider: self.search_provider,
            options: SearchOptions {
                max_results: self.max_results,
                search_domain_filter: self.search_domain_filter,
                max_tokens_per_page: self.max_tokens_per_page,
                country: self.country,
            },
            api_key: self.api_key,
            api_base: self.api_base,
            timeout,
            extra_headers: self.extra_headers.unwrap_or_default(),
            kwargs: self.kwargs,
        })
    }
}

// search_provider_cost_per_query's arguments, with LiteLLM's default of one query when the caller names none
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CostCall {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default = "default_number_of_queries")]
    pub(crate) number_of_queries: i64,
    #[serde(default)]
    pub(crate) optional_params: SearchOptionalParams,
}

// a single constant: the mutant that returns 1 again is equivalent, so the body is skipped rather than tested
#[cfg_attr(test, mutants::skip)]
const fn default_number_of_queries() -> i64 {
    1_i64
}

// litellm.ocr's bound request: the document as the caller shaped it, with its files in the blobs beside it
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OcrCall {
    pub(crate) model: String,
    pub(crate) document: OcrJson,
    #[serde(default)]
    pub(crate) api_key: Option<SecretString>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: Option<OcrObject>,
    #[serde(default)]
    pub(crate) optional_params: OcrObject,
    #[serde(default)]
    pub(crate) input_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) timeout_seconds: Option<f64>,
}

// project.rs classifies on `type` first: only a file document keeps its bytes, every other shape is the core's to decode
pub(crate) fn resolve_document(
    document: &OcrJson,
    blobs: &[Vec<u8>],
) -> Result<OcrDocumentInput, OcrError> {
    let OcrJson::Object(fields) = document else {
        return decode_document(document);
    };
    let kind = match fields.get("type") {
        Some(OcrJson::Text(kind)) => kind.as_str(),
        Some(
            OcrJson::Null
            | OcrJson::Bool(_)
            | OcrJson::Number(_)
            | OcrJson::List(_)
            | OcrJson::Object(_),
        )
        | None => {
            return Err(OcrError::RequestField {
                path: "document.type".to_owned(),
            });
        }
    };
    if kind != FILE_DOCUMENT_TYPE {
        return decode_document(document);
    }
    file_document(fields, blobs)
}

// A document field counts only when the caller sent a string; every other shape leaves it unset.
fn optional_text(fields: &OcrObject, key: &str) -> Option<String> {
    match fields.get(key) {
        Some(OcrJson::Text(text)) => Some(text.clone()),
        Some(
            OcrJson::Null
            | OcrJson::Bool(_)
            | OcrJson::Number(_)
            | OcrJson::List(_)
            | OcrJson::Object(_),
        )
        | None => None,
    }
}

fn file_document(fields: &OcrObject, blobs: &[Vec<u8>]) -> Result<OcrDocumentInput, OcrError> {
    let mime_type = optional_text(fields, "mime_type");
    let file = match fields.get("file") {
        Some(OcrJson::Null) | None => {
            return Err(OcrError::InvalidRequest(MISSING_FILE.to_owned()));
        }
        Some(OcrJson::Text(_)) => return Err(OcrError::InvalidRequest(BARE_STRING.to_owned())),
        Some(value) => value,
    };
    let OcrJson::Object(file_fields) = file else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    let Some(OcrJson::Object(marker)) = file_fields.get(FILE_MARKER_KEY) else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    marker_document(marker, blobs, mime_type)
}

// A marker names the file the Python layer read and the index of its bytes, so Rust never opens a path itself.
fn marker_document(
    marker: &OcrObject,
    blobs: &[Vec<u8>],
    mime_type: Option<String>,
) -> Result<OcrDocumentInput, OcrError> {
    let Some(OcrJson::Text(name)) = marker.get("name") else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    let Some(OcrJson::Number(index)) = marker.get("blob") else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    let Some(index) = index.as_u64().and_then(|value| usize::try_from(value).ok()) else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    let Some(bytes) = blobs.get(index) else {
        return Err(OcrError::InvalidRequest(UNSUPPORTED_FILE.to_owned()));
    };
    let content_type = optional_text(marker, "content_type");
    Ok(OcrDocumentInput::Bytes {
        bytes: bytes.clone(),
        file_name: Some(name.clone()).filter(|name| name != "file"),
        mime_type: mime_type.or(content_type),
    })
}

// What litellm.search hands back: the response and the hidden params LiteLLM keeps off the wire.
#[derive(Debug, Serialize)]
pub(crate) struct SearchAnswer<'answer> {
    pub(crate) response: &'answer SearchResponse,
    pub(crate) hidden_params: &'answer Object,
}

// What litellm.ocr hands back: the normalized response and the provider's own reply beside it.
#[derive(Debug, Serialize)]
pub(crate) struct OcrAnswer<'answer> {
    pub(crate) response: &'answer OCRResponse,
    pub(crate) provider_native_response: Option<&'answer OcrJson>,
}

// The cost answer is a tuple or LiteLLM's "model isn't mapped" failure, which Python raises as its own class.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(crate) enum CostAnswer {
    Cost {
        input_cost: Decimal,
        output_cost: Decimal,
    },
    Unmapped {
        message: String,
    },
}
