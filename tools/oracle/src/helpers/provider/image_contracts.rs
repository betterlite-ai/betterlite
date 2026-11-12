use betterlite_openai::api::image_contracts::records::{
    ImageClassEntry, ImageHeaders, ImageParams,
};
use betterlite_provider::api::image_contracts::edit::BaseImageEditConfig;
use serde::Deserialize;
use serde_json::json;

use edit::async_copy;
use errors::base_errors;
use fixtures::EditFixture;
use transport::transport_defaults;

mod edit;
mod errors;
mod fixtures;
mod transport;

const MODEL: &str = "dall-e-2";

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    TransportDefaults(TransportDefaults),
    EditAsyncTransformCopy(EditAsyncTransformCopy),
    EditClassConfiguration { entries: Vec<ImageClassEntry> },
    BaseErrors(BaseErrors),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransportDefaults {
    api_bases: Vec<Option<String>>,
    headers: ImageHeaders,
    edit_bases: Vec<Option<String>>,
    finalize_data: ImageParams,
    finalize_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditAsyncTransformCopy {
    scenarios: Vec<AsyncCopyScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AsyncCopyScenario {
    model: String,
    prompt: Option<String>,
    options: ImageParams,
    headers: ImageHeaders,
    litellm_params: ImageParams,
    raise_status: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaseErrors {
    message: String,
    status_code: u16,
    headers: ImageHeaders,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::TransportDefaults(fields) => Ok(transport_defaults(fields)),
            Input::EditAsyncTransformCopy(fields) => async_copy(fields),
            Input::EditClassConfiguration { entries } => Ok(json!({
                "config": <EditFixture as BaseImageEditConfig>::get_config(entries),
            })),
            Input::BaseErrors(fields) => Ok(base_errors(fields)),
        }
    }
}
