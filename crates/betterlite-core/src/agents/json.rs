use std::collections::BTreeMap;

#[expect(
    clippy::disallowed_types,
    reason = "Agent wire records intentionally preserve provider-defined JSON parameters"
)]
pub type AgentJson = serde_json::Value;

pub type SecurityRequirements = Vec<BTreeMap<String, Vec<String>>>;
