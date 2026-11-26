use std::collections::BTreeMap;

use betterlite_core::api::file_http_upload::service::Mode;
use betterlite_core::api::file_provider_read::ReadKwargs;
use betterlite_core::api::file_provider_write::{DeleteInput, UploadInput};
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::{ExpiresAfter, FileUpload};
use betterlite_sdk::call_files::CallFilesError;
use betterlite_sdk::call_files::files_from_payload;
use betterlite_sdk::call_setup::RequestMap;
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;

use self::params::{ParamsWire, read_kwargs, write_params};
use self::wire::{CreateWire, DeleteWire, ExpiresAfterWire, ListWire, RetrieveWire};

mod params;
mod tests;
mod wire;

#[derive(Debug)]
pub(crate) struct RetrieveCall {
    pub(crate) file_id: String,
    pub(crate) kwargs: ReadKwargs,
    pub(crate) trusted: TrustedCredentials,
}

#[derive(Debug)]
pub(crate) struct ListCall {
    pub(crate) purpose: Option<String>,
    pub(crate) kwargs: ReadKwargs,
    pub(crate) trusted: TrustedCredentials,
}

// one parsed request of any of the four calls, so the session's sync and async paths share one entry
pub(crate) enum Call {
    Create(Box<UploadInput<(), ()>>),
    Delete(Box<DeleteInput<(), ()>>),
    Retrieve(RetrieveCall),
    List(ListCall),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the file call payload is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the file call answer could not be written: {0}")]
    Answer(#[source] serde_json::Error),
    #[error("the file call's timeout is not usable: expected {0}")]
    TimeoutShapes(&'static str),
    #[error("the file call's timeout phases are not usable: {0}")]
    Phases(#[source] serde_json::Error),
    #[error("{0} is not a valid LlmProviders")]
    Provider(String),
    #[error("the {0} field has a value LiteLLM does not declare")]
    Field(&'static str),
    #[error("the file create call carries no file")]
    MissingFile,
    #[error(transparent)]
    File(#[from] CallFilesError),
}

impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}

// acreate_file forwards its declared default, so an explicit null there reaches LlmProviders(None)
pub(crate) fn parse_create(
    text: &str,
    blobs: &mut [Vec<u8>],
    mode: Mode,
) -> Result<UploadInput<(), ()>, ConvertError> {
    let wire: CreateWire = serde_json::from_str(text)?;
    if mode == Mode::Asynchronous && wire.custom_llm_provider == Some(None) {
        return Err(ConvertError::Provider(String::from("None")));
    }
    let file = file_upload(wire.file, blobs)?;
    let expires_after = wire.expires_after.map(expires_after).transpose()?;
    Ok(UploadInput {
        file,
        purpose: wire.purpose,
        expires_after,
        custom_llm_provider: wire.custom_llm_provider,
        extra_headers: wire.extra_headers,
        extra_body: wire.extra_body,
        params: write_params(&wire.params)?,
    })
}

pub(crate) fn parse_delete(text: &str) -> Result<DeleteInput<(), ()>, ConvertError> {
    let wire: DeleteWire = serde_json::from_str(text)?;
    Ok(DeleteInput {
        file_id: wire.file_id,
        model: wire.model,
        custom_llm_provider: named_provider(wire.custom_llm_provider)?,
        extra_headers: wire.extra_headers,
        extra_body: wire.extra_body,
        params: write_params(&wire.params)?,
    })
}

// the read wires differ only in file_id and purpose, and the rest of the mapping is the same
macro_rules! read_call {
    ($call:ident, $wire:expr, { $($own:ident: $value:expr),* $(,)? }) => {{
        let (kwargs, trusted) = read_parts(
            $wire.custom_llm_provider,
            $wire.params,
            $wire.extra_headers,
            $wire.extra_body,
        )?;
        Ok($call {
            $($own: $value,)*
            kwargs,
            trusted,
        })
    }};
}

pub(crate) fn parse_retrieve(text: &str) -> Result<RetrieveCall, ConvertError> {
    let wire: RetrieveWire = serde_json::from_str(text)?;
    read_call!(RetrieveCall, wire, {
        file_id: wire.file_id,
    })
}

pub(crate) fn parse_list(text: &str) -> Result<ListCall, ConvertError> {
    let wire: ListWire = serde_json::from_str(text)?;
    read_call!(ListCall, wire, {
        purpose: wire.purpose,
    })
}

fn read_parts(
    provider: Option<Option<String>>,
    params: ParamsWire,
    extra_headers: Option<BTreeMap<String, String>>,
    extra_body: Option<BTreeMap<String, String>>,
) -> Result<(ReadKwargs, TrustedCredentials), ConvertError> {
    let provider = named_provider(provider)?;
    read_kwargs(params, provider, extra_headers, extra_body)
}

// the upload's bytes arrive in a blob, and files_from_payload moves them out of it
fn file_upload(value: Json, blobs: &mut [Vec<u8>]) -> Result<FileUpload, ConvertError> {
    let mut payload = RequestMap::new();
    payload.insert(String::from("file"), value);
    let Some((_name, file)) = files_from_payload(&payload, blobs)?.pop() else {
        return Err(ConvertError::MissingFile);
    };
    Ok(FileUpload {
        filename: Some(file.name),
        content_type: file.content_type,
        data: file.bytes,
    })
}

// FileExpiresAfter declares anchor Literal["created_at"], and the encoder always writes that anchor
fn expires_after(wire: ExpiresAfterWire) -> Result<ExpiresAfter, ConvertError> {
    if wire.anchor != "created_at" {
        return Err(ConvertError::Field("expires_after.anchor"));
    }
    Ok(ExpiresAfter {
        seconds: wire.seconds,
    })
}

// an absent or null provider is the function's declared default; a null one the caller passed is LlmProviders(None)
fn named_provider(provider: Option<Option<String>>) -> Result<Option<String>, ConvertError> {
    match provider {
        None => Ok(None),
        Some(None) => Err(ConvertError::Provider(String::from("None"))),
        Some(Some(name)) => Ok(Some(name)),
    }
}
