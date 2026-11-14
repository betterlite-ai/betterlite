use betterlite_config::sections::alert_settings::types::SlackAlertingArgs;

pub mod decision;
pub mod error;
pub mod message;
pub mod ports;
pub mod record;

mod deployment;
pub mod entry;
mod region;
mod tests;

pub struct OutageDetector<'ports, Store, Deployments, Providers, ApiBase, Sender, Clock> {
    pub store: &'ports Store,
    // None stands for LiteLLM's llm_router being None, where both detectors return before any lookup
    pub deployments: Option<&'ports Deployments>,
    pub providers: &'ports Providers,
    pub api_base: &'ports ApiBase,
    pub sender: &'ports Sender,
    pub clock: &'ports Clock,
    pub args: &'ports SlackAlertingArgs,
}
