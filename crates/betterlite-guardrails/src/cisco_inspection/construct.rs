use betterlite_config::sections::cisco_verdict_contracts::Configured;
use reqwest::ClientBuilder;
use reqwest::redirect::Policy;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConstructError {
    #[error("could not build the Cisco inspection HTTP client: {source}")]
    Client {
        #[source]
        source: reqwest::Error,
    },
}

// Mirrors LiteLLM's CiscoAIDefenseGuardrail.__init__; both fields stay crate-visible so only new() sets the client's redirect policy.
#[derive(Debug, Clone)]
pub struct CiscoInspectionGuardrail {
    pub(crate) settings: Configured,
    pub(crate) client: reqwest::Client,
}

impl CiscoInspectionGuardrail {
    // LiteLLM's AsyncHTTPHandler always sends Cisco inspection calls with follow_redirects=False (docs/deviations/B3762.yaml).
    pub fn new(settings: Configured) -> Result<Self, ConstructError> {
        let client = ClientBuilder::new()
            .redirect(Policy::none())
            .build()
            .map_err(|source| ConstructError::Client { source })?;
        Ok(Self { settings, client })
    }
}

mod tests;
