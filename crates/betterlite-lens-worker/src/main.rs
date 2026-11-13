use std::collections::BTreeMap;
use std::env;
use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use tokio::runtime::Builder;

pub mod api;
pub mod failure;
pub mod http_api;
pub mod settings;
pub mod worker;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let environment = process_environment();
    let settings = match settings::WorkerSettings::resolve(&environment) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let policy = match ProxyPolicy::resolve(&ProxyTrust::default(), &environment) {
        Ok(policy) => Arc::new(policy),
        Err(error) => {
            tracing::error!("the proxy settings are not valid: {error}");
            return ExitCode::FAILURE;
        }
    };
    let client = match ProxiedClient::outbound(&policy) {
        Ok(client) => client,
        Err(error) => {
            tracing::error!("the HTTP client could not be built: {error}");
            return ExitCode::FAILURE;
        }
    };
    let api = http_api::HttpApi::new(&settings.litellm_url, settings.token, client);
    let worker = worker::LensWorker { api: Arc::new(api) };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("the runtime could not be built: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(worker.run_forever())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the binary is where the process environment enters, and every variable is handed to the settings and the proxy policy"
)]
fn process_environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (raw_name, raw_value) in env::vars_os() {
        let (Ok(name), Ok(value)) = (raw_name.into_string(), raw_value.into_string()) else {
            tracing::warn!("an environment variable is not UTF-8 and is ignored");
            continue;
        };
        environment.insert(name, value);
    }
    environment
}
