mod cases;
mod project;
mod tests;
mod transports;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_openai::output_limits::ErrorBody;
use betterlite_openai::output_limits::build::Recovery;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_tokenizer::count::runtime::runtime_counter;
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
        let config = ConfigLoader {
            lookup: BTreeMap::new(),
        }
        .load("")
        .map_err(|error| failure("ConfigError", None, error.to_string()))?;
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure("ProxyError", None, error.to_string()))?;
        let client = Arc::new(
            ProxiedClient::outbound(&Arc::new(policy))
                .map_err(|error| failure("ClientError", None, error.to_string()))?,
        );
        let tokens = runtime_counter(&config, &client, &BundledCatalog::default().get())
            .map_err(|error| failure("PromptCountError", None, error.to_string()))?;
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
