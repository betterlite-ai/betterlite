mod cases;
mod project;
mod tests;
mod transports;

use betterlite_openai::output_limits::ErrorBody;
use betterlite_openai::output_limits::build::Recovery;
use betterlite_openai::output_limits::tokens::TokenizerPromptTokens;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::Value;
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
        let tokens = TokenizerPromptTokens;
        // the same injected clock and id the Rust tests use, so the recovered shape stays comparable
        let recovery = Recovery {
            clock: || 1_700_000_000,
            next_id: || Uuid::from_u128(7),
            tokens: &tokens,
        };
        runtime.block_on(async {
            match input {
                Input::Drop(case) => cases::drop_case(case),
                Input::Marker(case) => Ok(Value::Bool(
                    ErrorBody::Http(case.message.as_bytes()).reaches_output_limit(),
                )),
                Input::Build(case) => cases::build_case(case, &recovery).await,
                Input::Azure(case) => cases::azure_case(case, &recovery).await,
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
