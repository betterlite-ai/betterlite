use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};

use super::{Preferences, RawPreferences, RawRouterConfig, RouterConfig};

impl RawRouterConfig {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<RouterConfig, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut available_models = Vec::with_capacity(self.available_models.len());
        for (position, model) in self.available_models.into_iter().enumerate() {
            let path = format!("{prefix}.available_models.{position}");
            available_models.push(resolve_string(model, &path, lookup)?);
        }
        let default_model = self
            .default_model
            .map(|model| resolve_string(model, &format!("{prefix}.default_model"), lookup))
            .transpose()?;
        let complexity_to_quality = self
            .complexity_to_quality
            .map(|declared| {
                tier_table(declared, &format!("{prefix}.complexity_to_quality"), lookup)
            })
            .transpose()?;
        Ok(RouterConfig {
            available_models,
            default_model,
            complexity_to_quality,
            extras: self.extras,
        })
    }
}

fn tier_table<L>(
    values: BTreeMap<String, RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<BTreeMap<String, i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (tier, value) in values {
        let path = format!("{prefix}.{tier}");
        reject_quoted_number(&value, &path)?;
        let Some(quality_tier) = resolve_number(Some(value), &path, lookup, parse_tier)? else {
            continue;
        };
        resolved.insert(tier, quality_tier);
    }
    Ok(resolved)
}

impl RawPreferences {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Preferences, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let tier_path = format!("{prefix}.quality_tier");
        reject_quoted_number(&self.quality_tier, &tier_path)?;
        let quality_tier = resolve_number(Some(self.quality_tier), &tier_path, lookup, parse_tier)?
            .ok_or_else(|| invalid_scalar(&tier_path))?;
        let mut keywords = Vec::with_capacity(self.keywords.len());
        for (position, keyword) in self.keywords.into_iter().enumerate() {
            let path = format!("{prefix}.keywords.{position}");
            keywords.push(resolve_string(keyword, &path, lookup)?);
        }
        let order = match self.order {
            Some(order) => {
                let path = format!("{prefix}.order");
                reject_quoted_number(&order, &path)?;
                resolve_number(Some(order), &path, lookup, parse_tier)?
            }
            None => None,
        };
        Ok(Preferences {
            quality_tier,
            keywords,
            order,
            extras: self.extras,
        })
    }
}

pub(super) fn money<L>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Option<Decimal>, ConfigError>
where
    L: EnvironmentLookup,
{
    resolve_number(value, path, lookup, parse_money)
}

fn parse_tier(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

// a quoted number is a string to the declared int; only an os.environ reference may be text
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
