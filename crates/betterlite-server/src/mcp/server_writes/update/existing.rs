use betterlite_db::mcp_stored_contracts::{JsonValue, MCPAuth};
use serde::Deserialize;

// Fields whose value belongs to the upstream just discovered or trusted; a new identity clears them.
pub(super) const AUTH_FLOW_SCOPED_FIELDS: [&str; 10] = [
    "issuer",
    "authorization_url",
    "token_url",
    "registration_url",
    "oauth2_flow",
    "dcr_bridge",
    "token_exchange_endpoint",
    "audience",
    "subject_token_type",
    "token_exchange_profile",
];

// Token-exchange settings that also live in dedicated columns; the column is authoritative.
pub(super) const TOKEN_EXCHANGE_COLUMNS: [&str; 4] = [
    "token_exchange_endpoint",
    "audience",
    "subject_token_type",
    "token_exchange_profile",
];

// The stored values the update path reads before deciding what to write.
#[derive(Deserialize)]
pub(super) struct ExistingRow {
    #[serde(default)]
    pub(super) server_name: Option<String>,
    #[serde(default)]
    pub(super) url: Option<String>,
    #[serde(default)]
    pub(super) auth_type: Option<MCPAuth>,
    #[serde(default)]
    pub(super) issuer: Option<String>,
    #[serde(default)]
    authorization_url: Option<String>,
    #[serde(default)]
    token_url: Option<String>,
    #[serde(default)]
    registration_url: Option<String>,
    #[serde(default)]
    oauth2_flow: Option<String>,
    #[serde(default)]
    dcr_bridge: Option<bool>,
    #[serde(default)]
    pub(super) token_exchange_endpoint: Option<String>,
    #[serde(default)]
    pub(super) audience: Option<String>,
    #[serde(default)]
    pub(super) subject_token_type: Option<String>,
    #[serde(default)]
    pub(super) token_exchange_profile: Option<String>,
    #[serde(default)]
    pub(super) credentials: Option<JsonValue>,
}

impl ExistingRow {
    // The column value as the prepared payload spells it, so a resubmitted value compares equal.
    pub(super) fn stored_field(&self, field: &str) -> Option<JsonValue> {
        match field {
            "issuer" => self.issuer.clone().map(JsonValue::String),
            "authorization_url" => self.authorization_url.clone().map(JsonValue::String),
            "token_url" => self.token_url.clone().map(JsonValue::String),
            "registration_url" => self.registration_url.clone().map(JsonValue::String),
            "oauth2_flow" => self.oauth2_flow.clone().map(JsonValue::String),
            "dcr_bridge" => self.dcr_bridge.map(JsonValue::Bool),
            "token_exchange_endpoint" => {
                self.token_exchange_endpoint.clone().map(JsonValue::String)
            }
            "audience" => self.audience.clone().map(JsonValue::String),
            "subject_token_type" => self.subject_token_type.clone().map(JsonValue::String),
            "token_exchange_profile" => self.token_exchange_profile.clone().map(JsonValue::String),
            _ => None,
        }
    }
}
