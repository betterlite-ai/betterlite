use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub(super) const ENV_MARKER: &str = "os.environ/";

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum RawFlag {
    Literal(bool),
    Text(String),
}

pub(super) fn resolve_flag<L>(value: RawFlag, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        RawFlag::Literal(flag) => Ok(flag),
        RawFlag::Text(text) => {
            // LiteLLM resolves os.environ/ strings before pydantic reads the field as a bool
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
    }
}
