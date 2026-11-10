use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::json::{Json, Object};
use crate::ocr_config::OCR_INLINE_MAX_BYTES;
use crate::ocr_document::{InlineDocument, OcrDocument};
use crate::ocr_error::OcrError;

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum DocumentIntelligenceRequest {
    UrlSource {
        #[serde(rename = "urlSource")]
        url_source: String,
    },
    Base64Source {
        #[serde(rename = "base64Source")]
        base64_source: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AnalyzeOperation {
    pub(crate) status: Option<OperationStatus>,
    #[serde(rename = "analyzeResult")]
    pub(crate) analyze_result: Option<AnalyzeResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct AnalyzeResult {
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) pages: Vec<AnalyzePage>,
    pub(crate) tables: Option<Vec<Object>>,
    #[serde(rename = "keyValuePairs")]
    pub(crate) key_value_pairs: Option<Vec<Object>>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AnalyzePage {
    #[serde(rename = "pageNumber", default)]
    pub(crate) page_number: Option<LaxInteger>,
    #[serde(default)]
    pub(crate) width: Option<FiniteNumber>,
    #[serde(default)]
    pub(crate) height: Option<FiniteNumber>,
    pub(crate) unit: Option<String>,
    #[serde(default)]
    pub(crate) lines: Vec<AnalyzeLine>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct AnalyzeLine {
    pub(crate) content: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum OperationStatus {
    Known(KnownStatus),
    Unknown(String),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum KnownStatus {
    Succeeded,
    Running,
    NotStarted,
    Failed,
}

impl OperationStatus {
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::Known(KnownStatus::Succeeded) => "succeeded",
            Self::Known(KnownStatus::Running) => "running",
            Self::Known(KnownStatus::NotStarted) => "notStarted",
            Self::Known(KnownStatus::Failed) => "failed",
            Self::Unknown(other) => other,
        }
    }
}

// LiteLLM's LaxI64 reads an integer, an integer string or a bool and rejects a whole float at 2^63
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "Json")]
pub(crate) struct LaxInteger(pub(crate) i64);

impl TryFrom<Json> for LaxInteger {
    type Error = PageFieldError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        lax_i64(&value).map(Self).ok_or(PageFieldError::Integer)
    }
}

// LiteLLM's FiniteF64 takes a finite number, a numeric string or a bool
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "FiniteWire")]
pub(crate) struct FiniteNumber(pub(crate) f64);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FiniteWire {
    Number(Number),
    Text(String),
    Bool(bool),
}

impl TryFrom<FiniteWire> for FiniteNumber {
    type Error = PageFieldError;

    fn try_from(value: FiniteWire) -> Result<Self, Self::Error> {
        let parsed = match value {
            FiniteWire::Number(number) => number.as_f64(),
            FiniteWire::Text(text) => text.trim().parse::<f64>().ok(),
            FiniteWire::Bool(flag) => Some(f64::from(flag)),
        };
        parsed
            .filter(|float| float.is_finite())
            .map(Self)
            .ok_or(PageFieldError::Number)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PageFieldError {
    #[error("expected an integer in the i64 range")]
    Integer,
    #[error("expected a finite number")]
    Number,
}

pub(super) fn build_request(
    document: &OcrDocument,
) -> Result<DocumentIntelligenceRequest, OcrError> {
    let source = document.source();
    if source.is_empty() {
        return Err(OcrError::MissingDocumentUrl);
    }
    Ok(match InlineDocument::parse(source)? {
        Some(inline) => DocumentIntelligenceRequest::Base64Source {
            base64_source: STANDARD.encode(inline.decode(OCR_INLINE_MAX_BYTES)?),
        },
        None => DocumentIntelligenceRequest::UrlSource {
            url_source: source.to_owned(),
        },
    })
}

fn lax_i64(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => integer_number(number),
        Json::Text(text) => decimal_integer(text.trim()),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn integer_number(number: &Number) -> Option<i64> {
    match (number.as_i64(), number.as_u64()) {
        (Some(signed), _) => Some(signed),
        (None, Some(unsigned)) => i64::try_from(unsigned).ok(),
        (None, None) => integral_in_range(number.as_f64()?),
    }
}

fn integral_in_range(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0_f64 {
        return None;
    }
    // Rust has no checked float-to-integer conversion, and an out-of-range value fails the parse
    format!("{value:.0}").parse().ok()
}

fn decimal_integer(text: &str) -> Option<i64> {
    let integer = match text.split_once('.') {
        Some((integer, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|byte| byte == b'0') {
                return None;
            }
            integer
        }
        None => text,
    };
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    let shaped = !integer.starts_with('_')
        && !integer.ends_with('_')
        && !integer.contains("__")
        && !digits.is_empty()
        && !digits.starts_with('_')
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_');
    if !shaped {
        return None;
    }
    integer.replace('_', "").parse().ok()
}
