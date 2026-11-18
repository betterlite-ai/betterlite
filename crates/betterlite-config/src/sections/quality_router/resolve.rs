use std::collections::BTreeMap;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};

use super::{
    Preferences, RawPreferences, RawRouterConfig, RouterConfig, default_complexity_to_quality,
};

impl RawRouterConfig {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<RouterConfig, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // the pinned defaults first, then each key the file declares overrides its field
        let mut config = RouterConfig {
            available_models: Vec::new(),
            default_model: None,
            complexity_to_quality: default_complexity_to_quality(),
            extras: self.extras,
        };
        config.available_models = resolve_strings(
            self.available_models,
            &format!("{prefix}.available_models"),
            lookup,
        )?;
        if let Some(model) = self.default_model {
            config.default_model = Some(resolve_string(
                model,
                &format!("{prefix}.default_model"),
                lookup,
            )?);
        }
        if let Some(declared) = self.complexity_to_quality {
            config.complexity_to_quality =
                tier_table(declared, &format!("{prefix}.complexity_to_quality"), lookup)?;
        }
        Ok(config)
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
        let Some(quality_tier) = tier_value(Some(value), &path, lookup)? else {
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
        Ok(Preferences {
            quality_tier: required_tier(
                Some(self.quality_tier),
                &format!("{prefix}.quality_tier"),
                lookup,
            )?,
            keywords: resolve_strings(self.keywords, &format!("{prefix}.keywords"), lookup)?,
            order: tier_value(self.order, &format!("{prefix}.order"), lookup)?,
            extras: self.extras,
        })
    }
}

// the quality tier has no default, unlike order, so an empty os.environ value is refused
fn required_tier<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    tier_value(value, path, lookup)?.ok_or_else(|| invalid_scalar(path))
}

fn resolve_strings<L>(
    values: Option<Vec<String>>,
    prefix: &str,
    lookup: &L,
) -> Result<Vec<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let declared = values.unwrap_or_default();
    let mut resolved = Vec::with_capacity(declared.len());
    for (position, value) in declared.into_iter().enumerate() {
        let path = format!("{prefix}.{position}");
        resolved.push(resolve_string(value, &path, lookup)?);
    }
    Ok(resolved)
}

fn tier_value<L>(
    value: Option<RawNumber>,
    path: &str,
    lookup: &L,
) -> Result<Option<i64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(value) = value else {
        return Ok(None);
    };
    reject_quoted_number(&value, path)?;
    resolve_number(Some(value), path, lookup, parse_tier)
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
