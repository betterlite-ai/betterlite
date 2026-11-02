mod tests;

use betterlite_server::public_endpoints::providers::catalog::provider_catalog;
use betterlite_server::public_endpoints::providers::endpoints::{
    build_endpoints_from_json, clean_display_name, load_bundled_endpoints,
};
use betterlite_server::public_endpoints::providers::error::ProvidersError;
use betterlite_server::public_endpoints::providers::get_supported_providers;
use serde::Deserialize;
use serde_json::{Value, json};
use std::future::Future;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GetSupportedProviders,
    GetProviderFields,
    GetSupportedEndpoints,
    GetSupportedEndpointsTwice,
    GetEndpointKeysInOrder,
    BuildEndpoints { raw: Value },
    CleanDisplayName { raw: String },
    ProviderCatalogRaw,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let provider_error = |error: ProvidersError| helper_error("ProvidersError", error);
        let json_error = |error: serde_json::Error| helper_error("SerializationError", error);
        let value = match input {
            Input::GetSupportedProviders => {
                let providers = block_on(get_supported_providers())?.0;
                serde_json::to_value(providers).map_err(json_error)?
            }
            Input::GetProviderFields | Input::ProviderCatalogRaw => {
                let catalog = provider_catalog().map_err(provider_error)?;
                serde_json::to_value(catalog).map_err(json_error)?
            }
            Input::GetSupportedEndpoints => {
                let response = load_bundled_endpoints().map_err(provider_error)?;
                serde_json::to_value(&response).map_err(json_error)?
            }
            Input::GetSupportedEndpointsTwice => {
                let first = load_bundled_endpoints().map_err(provider_error)?;
                let second = load_bundled_endpoints().map_err(provider_error)?;
                json!([first, second])
            }
            Input::GetEndpointKeysInOrder => {
                let response = load_bundled_endpoints().map_err(provider_error)?;
                let keys: Vec<String> = response
                    .endpoints
                    .iter()
                    .map(|entry| entry.key.clone())
                    .collect();
                serde_json::to_value(keys).map_err(json_error)?
            }
            Input::BuildEndpoints { raw } => {
                let bytes = serde_json::to_vec(raw).map_err(json_error)?;
                let endpoints = build_endpoints_from_json(&bytes).map_err(provider_error)?;
                serde_json::to_value(endpoints).map_err(json_error)?
            }
            Input::CleanDisplayName { raw } => Value::String(clean_display_name(raw)),
        };
        Ok(value)
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| helper_error("RuntimeError", error))
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
