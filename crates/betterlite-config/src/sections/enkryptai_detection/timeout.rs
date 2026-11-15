use std::num::ParseFloatError;
use std::time::{Duration, TryFromFloatSecsError};

use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub(super) const DEFAULT: Duration = Duration::from_secs(600);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum Seconds {
    Number(f64),
    Text(String),
    Boolean(bool),
    Empty(()),
}

#[derive(Debug, Error)]
enum TimeoutError {
    #[error("timeout must be numeric")]
    Number {
        #[source]
        source: ParseFloatError,
    },
    #[error("timeout must be a finite nonnegative duration")]
    Duration {
        #[source]
        source: TryFromFloatSecsError,
    },
}

impl Seconds {
    pub(super) fn into_duration<L>(self, path: &str, lookup: &L) -> Result<Duration, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let value = match self {
            Self::Empty(()) => return Ok(DEFAULT),
            Self::Number(number) => number,
            Self::Boolean(flag) => f64::from(u8::from(flag)),
            Self::Text(text) => {
                let resolved = resolve_string(text, path, lookup)?;
                if resolved.is_empty() {
                    return Ok(DEFAULT);
                }
                resolved
                    .trim()
                    .parse()
                    .map_err(|source| super::yaml_error(path, TimeoutError::Number { source }))?
            }
        };
        Duration::try_from_secs_f64(value)
            .map_err(|source| super::yaml_error(path, TimeoutError::Duration { source }))
    }
}
