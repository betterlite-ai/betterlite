use std::num::ParseFloatError;
use std::time::{Duration, TryFromFloatSecsError};

use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

const DEFAULT: Duration = Duration::from_secs(600);

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

pub(super) fn resolve<L>(
    raw: Option<Seconds>,
    path: &str,
    lookup: &L,
) -> Result<Duration, ConfigError>
where
    L: EnvironmentLookup,
{
    let seconds = match raw {
        None | Some(Seconds::Empty(())) => return Ok(DEFAULT),
        Some(Seconds::Text(text)) => {
            let resolved = resolve_string(text, path, lookup)?;
            if resolved.is_empty() {
                return Ok(DEFAULT);
            }
            resolved
                .trim()
                .parse::<f64>()
                .map_err(|source| super::yaml_error(path, TimeoutError::Number { source }))?
        }
        Some(Seconds::Number(number)) => number,
        Some(Seconds::Boolean(boolean)) => f64::from(u8::from(boolean)),
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| super::yaml_error(path, TimeoutError::Duration { source }))
}
