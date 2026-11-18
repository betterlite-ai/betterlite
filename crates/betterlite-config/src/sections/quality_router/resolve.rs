use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_money, resolve_number, resolve_string};

use super::{
    Preferences, RawPreferences, RawPrice, RawRouterConfig, RouterConfig,
    default_complexity_to_quality,
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
    if let Some(tier) = tier_value(value, path, lookup)? {
        return Ok(tier);
    }
    Err(invalid_scalar(path))
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
    let Some(declared) = value else {
        return Ok(None);
    };
    reject_quoted_number(&declared, path)?;
    resolve_number(Some(declared), path, lookup, parse_tier)
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

// the price arrives as written text, so a base-16, base-8 or base-2 integer resolves before the money parser
pub(super) fn resolve_price(
    value: Option<RawPrice>,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<Option<Decimal>, ConfigError> {
    let Some(price) = value else {
        return Ok(None);
    };
    // an os.environ reference reads its value through the shared number path
    if price.0.starts_with("os.environ/") {
        return resolve_number(Some(RawNumber::Text(price.0)), path, lookup, parse_money);
    }
    let text = integer_text(&price.0).unwrap_or(price.0);
    parse_money(&text, path).map(Some)
}

// YAML writes an integer in base 16, 8 or 2 with a prefix; the decimal text goes on to the money parser
fn integer_text(text: &str) -> Option<String> {
    let (sign, unsigned) = match text.split_at_checked(1) {
        Some((sign @ ("+" | "-"), rest)) => (sign, rest),
        _ => ("", text),
    };
    let (radix, digits) = match unsigned.get(..2) {
        Some("0x" | "0X") => (16, unsigned.get(2..)?),
        Some("0o" | "0O") => (8, unsigned.get(2..)?),
        Some("0b" | "0B") => (2, unsigned.get(2..)?),
        _ => return None,
    };
    if !digits
        .chars()
        .all(|character| character == '_' || character.to_digit(radix).is_some())
    {
        return None;
    }
    let value = Decimal::from_str_radix(&format!("{sign}{digits}"), radix).ok()?;
    Some(value.to_string())
}

fn invalid_scalar(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
