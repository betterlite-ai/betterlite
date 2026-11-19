use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use betterlite_logging::spend_sanitize::REDACTED_BY_LITELM;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

#[must_use]
pub fn restore_kill_switch(
    incoming: Option<&AgentKillSwitchConfig>,
    existing: Option<&AgentKillSwitchConfig>,
) -> Option<AgentKillSwitchConfig> {
    let current = incoming?;
    let Some(auth) = current.auth.as_ref() else {
        return Some(current.clone());
    };
    let mut restored = current.clone();
    restored.auth = Some(restore_auth(
        auth,
        existing.and_then(|config| config.auth.as_ref()),
    ));
    Some(restored)
}

fn restore_auth(
    incoming: &AgentKillSwitchAuth,
    existing: Option<&AgentKillSwitchAuth>,
) -> AgentKillSwitchAuth {
    match incoming {
        AgentKillSwitchAuth::Bearer { token } => AgentKillSwitchAuth::Bearer {
            token: restore_secret(token, stored_token(existing)),
        },
        AgentKillSwitchAuth::ApiKey {
            header_name,
            api_key,
        } => AgentKillSwitchAuth::ApiKey {
            header_name: header_name.clone(),
            api_key: restore_secret(api_key, stored_api_key(existing)),
        },
        AgentKillSwitchAuth::Basic { username, password } => AgentKillSwitchAuth::Basic {
            username: username.clone(),
            password: restore_secret(password, stored_password(existing)),
        },
    }
}

fn restore_secret(incoming: &SecretString, stored: Option<&SecretString>) -> SecretString {
    // the marker is public, but the value compared against it is a secret, so the comparison is constant time
    let is_marker = bool::from(
        incoming
            .expose_secret()
            .as_bytes()
            .ct_eq(REDACTED_BY_LITELM.as_bytes()),
    );
    if !is_marker {
        return incoming.clone();
    }
    stored.cloned().unwrap_or_else(|| SecretString::from(""))
}

const fn stored_token(existing: Option<&AgentKillSwitchAuth>) -> Option<&SecretString> {
    match existing {
        Some(AgentKillSwitchAuth::Bearer { token }) => Some(token),
        None | Some(AgentKillSwitchAuth::ApiKey { .. } | AgentKillSwitchAuth::Basic { .. }) => None,
    }
}

const fn stored_api_key(existing: Option<&AgentKillSwitchAuth>) -> Option<&SecretString> {
    match existing {
        Some(AgentKillSwitchAuth::ApiKey { api_key, .. }) => Some(api_key),
        None | Some(AgentKillSwitchAuth::Bearer { .. } | AgentKillSwitchAuth::Basic { .. }) => None,
    }
}

const fn stored_password(existing: Option<&AgentKillSwitchAuth>) -> Option<&SecretString> {
    match existing {
        Some(AgentKillSwitchAuth::Basic { password, .. }) => Some(password),
        None | Some(AgentKillSwitchAuth::Bearer { .. } | AgentKillSwitchAuth::ApiKey { .. }) => {
            None
        }
    }
}

mod tests;
