use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use betterlite_sdk::proxy_auth::clock::Clock;
use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};

mod azure;
mod handler;
mod oauth2;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Azure(azure::Azure),
    Oauth2(oauth2::Oauth2),
    Handler(handler::Handler),
}

#[derive(Clone, Debug, Default)]
struct Moment(Arc<AtomicI64>);

impl Clock for Moment {
    fn unix_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ProxyAuthError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Azure(request) => azure::run(request),
            Input::Oauth2(request) => runtime()?.block_on(oauth2::run(request)),
            Input::Handler(request) => handler::run(request),
        }
    }
}
