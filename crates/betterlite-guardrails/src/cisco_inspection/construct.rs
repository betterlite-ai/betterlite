use betterlite_config::sections::cisco_verdict_contracts::Configured;

// Mirrors LiteLLM's CiscoAIDefenseGuardrail.__init__; B3759's config section resolves Configured.
#[derive(Debug, Clone)]
pub struct CiscoInspectionGuardrail {
    pub settings: Configured,
    pub client: reqwest::Client,
}

mod tests;
