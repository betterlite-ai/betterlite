use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const AUDIENCE_VARIABLE: &str = "JWT_AUDIENCE";
pub const ISSUER_VARIABLE: &str = "JWT_ISSUER";

// JWTHandler._build_decode_kwargs reads os.getenv for these two and nothing else, so this section holds no YAML keys.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _no_keys: (),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub audience: Option<String>,
    pub issuer: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    // an absent variable stays None and an empty one stays Some(""), because only absence disables a check
    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            audience: lookup.lookup(AUDIENCE_VARIABLE),
            issuer: lookup.lookup(ISSUER_VARIABLE),
        })
    }
}

mod tests;
