use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[path = "mcp_bearer_credentials/defect.rs"]
pub mod defect;
use defect::SigningConfigDefect;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    mcp_session_token_signing: Option<RawSigningNode>,
}

// a wrong-shaped field falls back to IgnoredAny rather than invalidating the whole config document.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawSigningNode {
    Fields(RawSigningFields),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSigningFields {
    algorithm: String,
    kid: String,
    private_key: SecretString,
    #[serde(default)]
    previous_public_keys: Vec<RawPreviousKey>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPreviousKey {
    kid: String,
    public_key: String,
}

// a schema or reference defect is Invalid, so only the signing path fails closed, not the whole config load.
#[derive(Debug, Clone, Default)]
pub enum Settings {
    #[default]
    Absent,
    Resolved(ResolvedSigning),
    Invalid(SigningConfigDefect),
}

#[derive(Debug, Clone)]
pub struct ResolvedSigning {
    pub kid: String,
    pub private_key_pem: SecretString,
    pub previous_public_keys: Vec<ResolvedPreviousKey>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPreviousKey {
    pub kid: String,
    pub public_key_pem: SecretString,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(match self.general_settings.mcp_session_token_signing {
            None => Settings::Absent,
            Some(RawSigningNode::Other(_)) => Settings::Invalid(SigningConfigDefect::Malformed),
            Some(RawSigningNode::Fields(fields)) => resolve(&fields, lookup),
        })
    }
}

fn resolve<L>(fields: &RawSigningFields, lookup: &L) -> Settings
where
    L: EnvironmentLookup,
{
    if fields.algorithm != "RS256"
        || fields.kid.is_empty()
        || fields.private_key.expose_secret().is_empty()
    {
        return Settings::Invalid(SigningConfigDefect::Malformed);
    }
    if fields
        .previous_public_keys
        .iter()
        .any(|key| key.kid.is_empty() || key.public_key.is_empty())
    {
        return Settings::Invalid(SigningConfigDefect::Malformed);
    }
    let Some(private_key_pem) = resolve_material(fields.private_key.expose_secret(), lookup) else {
        return Settings::Invalid(SigningConfigDefect::UnresolvedPrivateKey);
    };
    let mut previous_public_keys = Vec::with_capacity(fields.previous_public_keys.len());
    let mut unresolved = Vec::new();
    for key in &fields.previous_public_keys {
        match resolve_material(&key.public_key, lookup) {
            Some(pem) => previous_public_keys.push(ResolvedPreviousKey {
                kid: key.kid.clone(),
                public_key_pem: SecretString::from(pem),
            }),
            None => unresolved.push(key.kid.clone()),
        }
    }
    if !unresolved.is_empty() {
        return Settings::Invalid(SigningConfigDefect::UnresolvedPreviousKeys { kids: unresolved });
    }
    Settings::Resolved(ResolvedSigning {
        kid: fields.kid.clone(),
        private_key_pem: SecretString::from(private_key_pem),
        previous_public_keys,
    })
}

fn resolve_material<L>(value: &str, lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    let Some(name) = value.strip_prefix("os.environ/") else {
        return Some(value.to_owned());
    };
    lookup.lookup(name)
}

#[path = "mcp_bearer_credentials/tests.rs"]
mod tests;
