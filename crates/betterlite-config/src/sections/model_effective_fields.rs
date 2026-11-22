use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const ENV_PREFIX: &str = "os.environ/";
const ENFORCE_PATH: &str = "general_settings.enforce_rpm_tpm_on_model_add";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    enforce_rpm_tpm_on_model_add: Option<RawFlag>,
}

// LiteLLM resolves os.environ/ references across the whole config before it reads the flag
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Literal(bool),
    Reference(String),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enforce_rpm_tpm_on_model_add: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let enforced = match self.general_settings.enforce_rpm_tpm_on_model_add {
            None => false,
            Some(RawFlag::Literal(flag)) => flag,
            Some(RawFlag::Reference(text)) => {
                let variable =
                    text.strip_prefix(ENV_PREFIX)
                        .ok_or_else(|| ConfigError::InvalidScalar {
                            path: ENFORCE_PATH.to_owned(),
                            value: false,
                        })?;
                betterlite_env::flag(variable, &lookup.lookup(variable).unwrap_or_default())?
                    .unwrap_or(false)
            }
        };
        Ok(Settings {
            enforce_rpm_tpm_on_model_add: enforced,
        })
    }
}

mod tests;
