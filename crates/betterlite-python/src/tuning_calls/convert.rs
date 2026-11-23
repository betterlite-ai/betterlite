use std::collections::BTreeMap;

use betterlite_config::loader::ConfigError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::resource_sdk_settings::{
    Raw, RawSettings, Settings as ConfiguredSettings,
};
use betterlite_core::api::fine_tuning_cancel_call::{FineTuningCancelCall, FineTuningCancelTarget};
use betterlite_core::api::fine_tuning_create::{CreateFineTuningJobCall, FineTuningCreateTarget};
use betterlite_core::api::fine_tuning_read::{FineTuningReadInput, ReadTarget};
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::decisions::Json;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use pyo3::PyErr;
use pyo3::exceptions::PyValueError;
use serde::Deserialize;

use self::failures::wire_failure;
use self::requests::{
    azure_cancel_request, azure_create_request, azure_read_request, openai_cancel_request,
    openai_create_request, openai_read_request,
};
use super::error::WireError;

mod azure;
pub(crate) mod failures;
mod requests;
mod tests;

// The text a native method takes: the call's arguments and the module settings snapshot.
#[derive(Debug, Deserialize)]
pub(crate) struct CreateCall {
    pub(crate) call: CreateFineTuningJobCall,
    #[serde(default)]
    pub(crate) settings: SettingsWire,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReadCall {
    pub(crate) input: FineTuningReadInput,
    #[serde(default)]
    pub(crate) settings: SettingsWire,
}

#[derive(Deserialize)]
pub(crate) struct CancelCall {
    pub(crate) call: FineTuningCancelCall,
    #[serde(default)]
    pub(crate) settings: SettingsWire,
}

// The module settings litellm exposes, read by the same config section the file settings use.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    #[serde(flatten)]
    raw: RawSettings,
}

impl SettingsWire {
    // The section fills every environment-backed field, so the module values and the snapshot meet once.
    pub(crate) fn configured(
        self,
        environment: &BTreeMap<String, String>,
    ) -> Result<ConfiguredSettings, ConfigError> {
        Raw { settings: self.raw }.convert(environment)
    }
}

// the one boundary every native method crosses: the text into the shape that method takes
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, WireError>
where
    T: Deserialize<'text>,
{
    serde_json::from_str(text).map_err(|source| WireError::Json { source })
}

// A malformed request text is the caller's bug, so it raises ValueError rather than a LiteLLM class.
pub(crate) fn request_error(source: WireError) -> PyErr {
    if let WireError::Json { .. } = source {
        return PyValueError::new_err(source.to_string());
    }
    wire_failure(source).to_pyerr()
}

pub(crate) fn create_request(
    target: FineTuningCreateTarget,
    integrations: Option<&[Json]>,
) -> Result<(OutboundRequest, bool), WireError> {
    match target {
        FineTuningCreateTarget::OpenAi(call) => {
            Ok((openai_create_request(&call, integrations)?, false))
        }
        FineTuningCreateTarget::Azure(call) => {
            Ok((azure_create_request(&call, integrations)?, true))
        }
        // the vertex_ai provider call belongs to B1384, so this family answers the gap rather than a request
        FineTuningCreateTarget::Vertex(_) => Err(WireError::Vertex),
    }
}

pub(crate) fn read_request(target: ReadTarget<'_>) -> Result<OutboundRequest, WireError> {
    match target {
        ReadTarget::OpenAi(call) => openai_read_request(&call),
        ReadTarget::Azure(call) => azure_read_request(&call),
    }
}

pub(crate) fn cancel_request(
    target: FineTuningCancelTarget<'_>,
) -> Result<OutboundRequest, WireError> {
    match target {
        FineTuningCancelTarget::OpenAi(call) => openai_cancel_request(&call),
        FineTuningCancelTarget::Azure(call) => azure_cancel_request(&call),
    }
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
) -> Result<(), WireError> {
    let parsed =
        HeaderValue::from_str(value).map_err(|source| WireError::Header { name, source })?;
    headers.insert(HeaderName::from_static(name), parsed);
    Ok(())
}
