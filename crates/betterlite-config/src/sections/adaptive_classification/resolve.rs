use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};

use super::{Preferences, RawPreferences, RawWeights, Weights};

impl RawWeights {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Weights, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Weights {
            quality: weight(self.quality, &format!("{prefix}.quality"), lookup)?,
            economy: weight(self.cost, &format!("{prefix}.cost"), lookup)?,
        })
    }
}

impl RawPreferences {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Preferences, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let tier_path = format!("{prefix}.model_info.adaptive_router_preferences.quality_tier");
        reject_quoted_number(&self.quality_tier, &tier_path)?;
        let quality_tier = resolve_number(Some(self.quality_tier), &tier_path, lookup, parse_tier)?
            .ok_or_else(|| invalid_scalar(&tier_path))?;
        let strengths_path = format!("{prefix}.model_info.adaptive_router_preferences.strengths");
        let strengths = self
            .strengths
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(position, strength)| {
                resolve_string(strength, &format!("{strengths_path}.{position}"), lookup)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Preferences {
            quality_tier,
            strengths,
        })
    }
}

fn weight<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(number) = &value {
        reject_quoted_number(number, path)?;
    }
    resolve_number(value, path, lookup, parse_float)
}

fn parse_tier(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

// a quoted number is a string to the declared float and int; only an os.environ reference may be text
fn reject_quoted_number(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    match value {
        RawNumber::Text(text) if !text.starts_with("os.environ/") => Err(invalid_scalar(path)),
        RawNumber::Integer(_)
        | RawNumber::Float(_)
        | RawNumber::Boolean(_)
        | RawNumber::Text(_) => Ok(()),
    }
}

fn invalid_scalar(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
