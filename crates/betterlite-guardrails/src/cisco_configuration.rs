use betterlite_config::sections::cisco_verdict_contracts::Configured;

pub mod choice;
pub mod event_hooks;
pub mod payload;
pub mod records;

// Typed form of LiteLLM's CiscoAIDefenseGuardrailConfigModel.
pub type CiscoConfigModel = Configured;

mod tests;
