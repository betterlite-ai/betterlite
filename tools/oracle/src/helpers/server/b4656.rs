mod cache_backend;
mod cached_store;
mod codec;
mod dual_cache;
mod refresh_coordinator;
mod refreshing_store;
mod store;
mod support;
mod token_fields;
mod v2_store;

use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Token(token_fields::Input),
    Store(store::Input),
    Codec(codec::Input),
    Coordinator(refresh_coordinator::Input),
    #[serde(rename = "v2")]
    V2(v2_store::Input),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Token(payload) => token_fields::run(payload),
            Input::Store(payload) => store::run(payload),
            Input::Codec(payload) => codec::run(payload),
            Input::Coordinator(payload) => refresh_coordinator::run(payload),
            Input::V2(payload) => v2_store::run(payload),
        }
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}
