use std::str::FromStr as _;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use secrecy::SecretString;

type Lookup<'lookup> = &'lookup dyn EnvironmentLookup;

pub(super) fn resolve_secret(
    value: Option<String>,
    path: &str,
    lookup: Lookup<'_>,
) -> Result<Option<SecretString>, ConfigError> {
    let Some(text) = value else {
        return Ok(None);
    };
    let secret = SecretString::from(resolve_string(text, path, lookup)?);
    Ok(Some(secret))
}

pub(super) fn resolve_string(
    value: String,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<String, ConfigError> {
    let Some(name) = value.strip_prefix("os.environ/") else {
        return Ok(value);
    };
    lookup
        .lookup(name)
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path: path.to_owned(),
        })
}

pub(super) fn parse_float(text: &str, path: &str) -> Result<f64, ConfigError> {
    let parsed = text.parse::<f64>();
    parsed.map_or_else(
        |source| {
            Err(ConfigError::InvalidFloat {
                path: path.to_owned(),
                source,
            })
        },
        Ok,
    )
}

pub(super) fn parse_integer(text: &str, path: &str) -> Result<u32, ConfigError> {
    match u32::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        }),
    }
}

pub(super) fn resolve_number<T, F>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
    parser: F,
) -> Result<Option<T>, ConfigError>
where
    F: Fn(&str, &str) -> Result<T, ConfigError>,
{
    let Some(number) = value else {
        return Ok(None);
    };
    let text = number_text(number, path, lookup)?;
    Ok(Some(parser(&text, path)?))
}

fn number_text(
    value: RawNumber,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<String, ConfigError> {
    match value {
        RawNumber::Integer(integer) => Ok(integer.to_string()),
        RawNumber::Float(float) => Ok(float.to_string()),
        RawNumber::Text(text) => resolve_string(text, path, lookup),
        RawNumber::Boolean(boolean) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: boolean,
        }),
    }
}
