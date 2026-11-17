mod cases;
mod project;
mod tests;
mod transports;

use std::time::Duration;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use tokio::runtime::Builder;
use uuid::Uuid;

use cases::{AzureCase, BuildCase, DropCase, MarkerCase};
use project::failure;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("OSError", None, error.to_string()))?;
        runtime.block_on(async {
            match input {
                Input::Drop(case) => cases::drop_case(case),
                Input::Marker(case) => Ok(cases::marker_case(case)),
                Input::Build(case) => cases::build_case(case).await,
                Input::Azure(case) => cases::azure_case(case).await,
            }
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Drop(DropCase),
    Marker(MarkerCase),
    Build(BuildCase),
    Azure(AzureCase),
}

// the same injected clock and id the Rust tests use, so the recovered shape stays comparable
pub(crate) const fn fixed_clock() -> u64 {
    1_700_000_000
}

pub(crate) const fn fixed_id() -> Uuid {
    Uuid::from_u128(7)
}

pub(crate) const fn no_elapsed() -> Duration {
    Duration::ZERO
}
