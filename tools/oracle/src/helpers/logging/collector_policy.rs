use betterlite_config::collector::{
    AddressError, CollectorAddress, CollectorSettings, load_spend_traceback_policy,
    parse_collector_address,
};
use betterlite_logging::spend_error::should_suppress_tracebacks;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::watch;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    addresses: Vec<String>,
    #[serde(default)]
    settings: Vec<CollectorSettings>,
    policies: Vec<Policy>,
}
#[derive(Deserialize)]
struct Policy {
    value: Option<String>,
    debug: bool,
}

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let addresses: Vec<_> = input
            .addresses
            .iter()
            .map(|address| match parse_collector_address(address) {
                Ok(CollectorAddress::Unix { path }) => json!({"kind":"unix", "path":path}),
                Ok(CollectorAddress::Tcp { host, port }) => {
                    json!({"kind":"tcp", "host":host, "port":port})
                }
                Err(AddressError::Syntax(_)) => json!({"kind":"syntax"}),
                Err(AddressError::Invalid | AddressError::RemoteHost) => json!({"kind":"invalid"}),
            })
            .collect();
        let produces: Vec<_> = input
            .settings
            .iter()
            .map(|settings| settings.enabled && settings.job_role.as_deref() != Some("collector"))
            .collect();
        let suppress: Vec<_> = input
            .policies
            .iter()
            .map(|policy| {
                let settings =
                    load_spend_traceback_policy(policy.value.as_deref()).unwrap_or_default();
                let (_, snapshot) = watch::channel(settings);
                should_suppress_tracebacks(&snapshot, policy.debug)
            })
            .collect();
        Ok(json!({"addresses":addresses,"produces":produces,"suppress":suppress}))
    }
}
