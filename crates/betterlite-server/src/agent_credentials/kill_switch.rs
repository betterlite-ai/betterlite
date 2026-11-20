use std::mem;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::kill_switch::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use secrecy::ExposeSecret as _;

use crate::agent_credentials::error::AgentCredentialsError;
use crate::kill_switch_management::restore::restore_kill_switch;

#[must_use]
pub fn parse_agent_kill_switch(value: Option<&AgentJson>) -> Option<AgentKillSwitchConfig> {
    let value = value?;
    match value {
        AgentJson::Null => None,
        AgentJson::String(text) => serde_json::from_str::<AgentJson>(text)
            .ok()
            .and_then(|parsed| serde_json::from_value::<AgentKillSwitchConfig>(parsed).ok()),
        AgentJson::Bool(_) | AgentJson::Number(_) | AgentJson::Array(_) | AgentJson::Object(_) => {
            serde_json::from_value::<AgentKillSwitchConfig>(value.clone()).ok()
        }
    }
}

pub fn serialize_agent_kill_switch(
    incoming: Option<&AgentKillSwitchConfig>,
    existing: Option<&AgentJson>,
) -> Result<String, AgentCredentialsError> {
    let stored = restore_kill_switch(incoming, parse_agent_kill_switch(existing).as_ref());
    let Some(stored) = stored else {
        return Ok(String::from("null"));
    };
    let mut value = serde_json::to_value(&stored).map_err(AgentCredentialsError::Serialize)?;
    if let Some(auth) = stored.auth.as_ref() {
        write_auth_secret(&mut value, auth);
    }
    strip_nul(&mut value);
    serde_json::to_string(&value).map_err(AgentCredentialsError::Serialize)
}

fn write_auth_secret(value: &mut AgentJson, auth: &AgentKillSwitchAuth) {
    let Some(auth_fields) = value.get_mut("auth").and_then(AgentJson::as_object_mut) else {
        return;
    };
    let (field, secret) = match auth {
        AgentKillSwitchAuth::Bearer { token } => ("token", token),
        AgentKillSwitchAuth::ApiKey { api_key, .. } => ("api_key", api_key),
        AgentKillSwitchAuth::Basic { password, .. } => ("password", password),
    };
    auth_fields.insert(
        String::from(field),
        AgentJson::String(String::from(secret.expose_secret())),
    );
}

// Postgres rejects \u0000 in a jsonb column, so the stored text carries none
fn strip_nul(value: &mut AgentJson) {
    match value {
        AgentJson::String(text) => *text = text.replace('\0', ""),
        AgentJson::Array(items) => {
            for item in items {
                strip_nul(item);
            }
        }
        AgentJson::Object(fields) => {
            let entries = mem::take(fields);
            for (key, mut nested) in entries {
                strip_nul(&mut nested);
                fields.insert(key.replace('\0', ""), nested);
            }
        }
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Number(_) => {}
    }
}

mod tests;
