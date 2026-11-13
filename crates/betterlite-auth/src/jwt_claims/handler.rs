use std::sync::Arc;

use super::settings::JwtAuthSettings;

// the registered-agent port: jwt_keys owns the concrete lookups, this handler only carries the binding
pub trait AgentLookup: Send + Sync {}

#[derive(Default)]
pub struct JwtHandler {
    pub settings: JwtAuthSettings,
    pub trusted_issuers: Vec<String>,
    pub leeway: i64,
    pub agent_lookup: Option<Arc<dyn AgentLookup>>,
}

impl JwtHandler {
    pub fn update_environment(
        &mut self,
        settings: JwtAuthSettings,
        trusted_issuers: Vec<String>,
        leeway: i64,
    ) {
        self.settings = settings;
        self.trusted_issuers = trusted_issuers;
        self.leeway = leeway;
    }

    #[must_use]
    pub fn bind_agent_lookup(mut self, lookup: Arc<dyn AgentLookup>) -> Self {
        self.agent_lookup = Some(lookup);
        self
    }
}
