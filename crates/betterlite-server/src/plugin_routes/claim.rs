use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::loader::startup::process_environment;
use hmac::{Hmac, KeyInit as _, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use super::error::PluginError;
use super::fernet::{self, FernetKey};

pub const SALT_VARIABLE: &str = "LITELLM_SALT_KEY";
pub const CLAIM_TTL_SECONDS: u64 = 30;

const KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Claim {
    pub plugin: String,
    pub user_id: String,
    pub user_role: String,
    pub exp: i64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClaimMaterial<'value> {
    pub(crate) salt: &'value str,
    pub(crate) plugin: &'value str,
    pub(crate) user_id: Option<&'value str>,
    pub(crate) user_role: Option<&'value str>,
    pub(crate) now: u64,
    pub(crate) iv: [u8; fernet::IV_BYTES],
}

// The caller of the dashboard gets a claim bound to one plugin; it carries identity, never a litellm key.
pub fn issue_plugin_session_claim(
    plugin_name: &str,
    user_id: Option<&str>,
    user_role: Option<&str>,
) -> Result<String, PluginError> {
    let salt = process_environment()
        .get(SALT_VARIABLE)
        .cloned()
        .unwrap_or_default();
    let material = ClaimMaterial {
        salt: salt.as_str(),
        plugin: plugin_name,
        user_id,
        user_role,
        now: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
        // a v4 UUID carries 122 random bits, enough for a one-shot IV and infallible to mint
        iv: Uuid::new_v4().into_bytes(),
    };
    issue_claim(&material)
}

pub fn verify_plugin_session_claim(
    plugin_name: &str,
    ciphertext: &str,
) -> Result<Claim, PluginError> {
    let salt = process_environment()
        .get(SALT_VARIABLE)
        .cloned()
        .unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    verify_claim_at(salt.as_str(), plugin_name, ciphertext, now)
}

pub(crate) fn issue_claim(material: &ClaimMaterial<'_>) -> Result<String, PluginError> {
    let claim = Claim {
        plugin: material.plugin.to_owned(),
        user_id: material.user_id.unwrap_or_default().to_owned(),
        user_role: material.user_role.unwrap_or_default().to_owned(),
        exp: i64::try_from(material.now.saturating_add(CLAIM_TTL_SECONDS)).unwrap_or(i64::MAX),
    };
    let key = plugin_key(material.salt, material.plugin)?;
    let plaintext = serde_json::to_vec(&claim).unwrap_or_default();
    Ok(fernet::seal(&key, &plaintext, material.now, material.iv))
}

pub(crate) fn verify_claim_at(
    salt: &str,
    plugin_name: &str,
    ciphertext: &str,
    now: u64,
) -> Result<Claim, PluginError> {
    let key = plugin_key(salt, plugin_name)?;
    let plaintext = fernet::open(&key, ciphertext, CLAIM_TTL_SECONDS, now)
        .map_err(|_error| PluginError::InvalidClaim)?;
    let claim: Claim =
        serde_json::from_slice(&plaintext).map_err(|_error| PluginError::InvalidClaim)?;
    if claim.plugin != plugin_name {
        return Err(PluginError::AudienceMismatch);
    }
    let now_seconds = i64::try_from(now).unwrap_or(i64::MAX);
    if claim.exp < now_seconds {
        return Err(PluginError::ClaimExpired);
    }
    Ok(claim)
}

fn plugin_key(salt: &str, plugin_name: &str) -> Result<FernetKey, PluginError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(salt.as_bytes())
        .map_err(|source| PluginError::ClaimKey { source })?;
    mac.update(plugin_name.as_bytes());
    let material = <[u8; KEY_BYTES]>::from(mac.finalize().into_bytes());
    Ok(FernetKey::from_material(material))
}

#[cfg(test)]
mod tests;
