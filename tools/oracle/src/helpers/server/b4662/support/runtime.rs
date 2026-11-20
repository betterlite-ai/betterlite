use std::future::Future;

use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::value_error;

pub(in super::super) fn block_on<T, F>(future: F) -> HelperResult
where
    T: serde::Serialize,
    F: Future<Output = Result<T, Box<HelperError>>>,
{
    let runtime = runtime()?;
    let value = runtime.block_on(future)?;
    serde_json::to_value(value).map_err(|error| value_error(error.to_string()))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| value_error(error.to_string()))
}
