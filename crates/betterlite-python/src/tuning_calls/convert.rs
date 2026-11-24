use betterlite_config::sections::resource_sdk_settings::RawSettings;
use betterlite_core::api::fine_tuning_cancel_call::FineTuningCancelCall;
use betterlite_core::api::fine_tuning_create::CreateFineTuningJobCall;
use betterlite_core::api::fine_tuning_read::FineTuningReadInput;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use super::error::WireError;

mod azure;
pub(crate) mod failures;
pub(crate) mod requests;
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

// The caller's SDK client is not Debug, so this call record carries no derived one.
#[derive(Deserialize)]
pub(crate) struct CancelCall {
    pub(crate) call: FineTuningCancelCall,
    #[serde(default)]
    pub(crate) settings: SettingsWire,
}

// One runner decodes all three call shapes, and the boxes keep every variant the same size.
pub(crate) enum NativeCall {
    Create(Box<CreateCall>),
    Read(Box<ReadCall>),
    Cancel(Box<CancelCall>),
}

// The module settings litellm exposes, read by the same config section the file settings use.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsWire {
    #[serde(flatten)]
    pub(crate) raw: RawSettings,
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
