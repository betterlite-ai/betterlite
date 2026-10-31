use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::rag::document::ProviderIdentity as _;
use betterlite_core::rag::document::pdf::extract_text_from_pdf;
use betterlite_core::rag::document::provider::get_ingestion_class;
use betterlite_core::rag::document::splitter::{RecursiveCharacterTextSplitter, TextSplitter as _};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum Input {
    Split {
        chunk_size: i64,
        chunk_overlap: i64,
        #[serde(default)]
        separators: Option<Vec<String>>,
        text: String,
    },
    ExtractPdf {
        pdf_base64: String,
    },
    ProviderLookup {
        provider: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Split {
                chunk_size,
                chunk_overlap,
                separators,
                text,
            } => split(*chunk_size, *chunk_overlap, separators.clone(), text),
            Input::ExtractPdf { pdf_base64 } => extract_pdf(pdf_base64),
            Input::ProviderLookup { provider } => provider_lookup(provider),
        }
    }
}

fn split(
    chunk_size: i64,
    chunk_overlap: i64,
    separators: Option<Vec<String>>,
    text: &str,
) -> HelperResult {
    let splitter = RecursiveCharacterTextSplitter::new(chunk_size, chunk_overlap, separators)
        .map_err(|error| value_error(&error))?;
    let chunks = splitter.split_text(text);
    Ok(Value::Array(
        chunks.into_iter().map(Value::String).collect(),
    ))
}

fn extract_pdf(pdf_base64: &str) -> HelperResult {
    let bytes = BASE64
        .decode(pdf_base64)
        .map_err(|error| value_error(&error))?;
    let text = extract_text_from_pdf(&bytes).map_or(Value::Null, Value::String);
    Ok(serde_json::json!({ "text": text }))
}

fn provider_lookup(provider: &str) -> HelperResult {
    let resolved = get_ingestion_class(provider).map_err(|error| value_error(&error))?;
    Ok(Value::String(resolved.ingestion_class_name().to_owned()))
}

fn value_error(error: &dyn std::fmt::Display) -> Box<HelperError> {
    let message = error.to_string();
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
