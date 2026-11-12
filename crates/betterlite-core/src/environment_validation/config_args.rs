use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::error::EnvironmentValidationError;

// LiteLLM reads any size; a config file past this is refused so a bad path cannot fill memory.
pub const MAX_CONFIG_BYTES: u64 = 10 * 1024 * 1024;

pub fn read_config_args(
    config_path: &Path,
) -> Result<IndexMap<String, Box<RawValue>>, EnvironmentValidationError> {
    let read_error = |source| EnvironmentValidationError::ReadConfig {
        path: config_path.to_path_buf(),
        source,
    };
    let file = File::open(config_path).map_err(read_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if u64::try_from(bytes.len()).is_ok_and(|length| length > MAX_CONFIG_BYTES) {
        return Err(EnvironmentValidationError::ConfigTooLarge {
            path: config_path.to_path_buf(),
            limit: MAX_CONFIG_BYTES,
        });
    }
    serde_json::from_slice(&bytes).map_err(|source| EnvironmentValidationError::ParseConfig {
        path: config_path.to_path_buf(),
        source,
    })
}
