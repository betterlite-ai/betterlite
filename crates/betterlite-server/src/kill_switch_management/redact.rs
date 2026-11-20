use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use betterlite_logging::spend_sanitize::REDACTED_BY_LITELM;
use secrecy::SecretString;

#[must_use]
pub fn redact_kill_switch(config: Option<&AgentKillSwitchConfig>) -> Option<AgentKillSwitchConfig> {
    let current = config?;
    let Some(auth) = current.auth.as_ref() else {
        return Some(current.clone());
    };
    let mut redacted = current.clone();
    redacted.auth = Some(redact_auth(auth));
    Some(redacted)
}

fn redact_auth(auth: &AgentKillSwitchAuth) -> AgentKillSwitchAuth {
    let marker = SecretString::from(REDACTED_BY_LITELM);
    match auth {
        AgentKillSwitchAuth::Bearer { .. } => AgentKillSwitchAuth::Bearer { token: marker },
        AgentKillSwitchAuth::ApiKey { header_name, .. } => AgentKillSwitchAuth::ApiKey {
            header_name: header_name.clone(),
            api_key: marker,
        },
        AgentKillSwitchAuth::Basic { username, .. } => AgentKillSwitchAuth::Basic {
            username: username.clone(),
            password: marker,
        },
    }
}

mod tests;
