use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

const ENV_MARKER: &str = "os.environ/";

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawFlag {
    Literal(bool),
    Text(String),
}

impl RawFlag {
    // LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
    pub(super) fn into_bool<L>(self, path: &str, lookup: &L) -> Result<Option<bool>, ConfigError>
    where
        L: EnvironmentLookup,
    {
        match self {
            Self::Literal(value) => Ok(Some(value)),
            Self::Text(text) => decode_flag(text, path, lookup),
        }
    }
}

// os.environ/NAME reads NAME from the lookup; any other text is already the literal
fn decode_flag<L>(text: String, path: &str, lookup: &L) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
    let resolved = resolve_string(text, path, lookup)?;
    Ok(betterlite_env::flag(&name, &resolved)?)
}
