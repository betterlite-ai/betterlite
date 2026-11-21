use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, parse_integer, resolve_number, resolve_string};

use super::{
    DEFAULT_MATCH_THRESHOLD, DEFAULT_SEMANTIC_KEYWORD_MATCHING, DEFAULT_STALL_ESCALATION_ENABLED,
    DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD, DEFAULT_STALL_ESCALATION_WINDOW, KeywordMatching,
    KeywordTierRule, RawFlag, RawKeywordTierRule,
};

const ENV_MARKER: &str = "os.environ/";

pub(super) fn keyword_rules(
    raw: Option<Vec<RawKeywordTierRule>>,
    prefix: &str,
) -> Result<Option<Vec<KeywordTierRule>>, ConfigError> {
    let Some(rules) = raw else {
        return Ok(None);
    };
    let mut compiled = Vec::with_capacity(rules.len());
    for (index, rule) in rules.into_iter().enumerate() {
        compiled.push(normalize_rule(
            rule,
            &format!("{prefix}.keyword_tier_rules.{index}"),
        )?);
    }
    Ok(Some(compiled))
}

pub(super) fn stall<L>(
    enabled: Option<RawFlag>,
    window_value: Option<RawNumber>,
    repeat: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<(bool, u32, u32), ConfigError>
where
    L: EnvironmentLookup,
{
    let enabled = resolve_flag(
        enabled,
        &format!("{prefix}.stall_escalation_enabled"),
        lookup,
    )?
    .unwrap_or(DEFAULT_STALL_ESCALATION_ENABLED);
    Ok((
        enabled,
        window(
            window_value,
            &format!("{prefix}.stall_escalation_window"),
            lookup,
        )?,
        repeat_threshold(
            repeat,
            &format!("{prefix}.stall_escalation_repeat_threshold"),
            lookup,
        )?,
    ))
}

pub(super) fn semantic<L>(
    declared: Option<RawFlag>,
    model: Option<String>,
    threshold_value: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<(KeywordMatching, Option<String>, f64), ConfigError>
where
    L: EnvironmentLookup,
{
    let enabled = resolve_flag(
        declared,
        &format!("{prefix}.semantic_keyword_matching"),
        lookup,
    )?
    .unwrap_or(DEFAULT_SEMANTIC_KEYWORD_MATCHING);
    Ok((
        if enabled {
            KeywordMatching::Semantic
        } else {
            KeywordMatching::Lexical
        },
        optional_text(model, &format!("{prefix}.embedding_model"), lookup)?,
        threshold(
            threshold_value,
            &format!("{prefix}.match_threshold"),
            lookup,
        )?,
    ))
}

// KeywordTierRule._normalize_keywords and _coerce_tier: strip, drop blanks, require one keyword and a tier
fn normalize_rule(raw: RawKeywordTierRule, path: &str) -> Result<KeywordTierRule, ConfigError> {
    let Some(keywords) = raw.keywords else {
        return Err(invalid(path));
    };
    let mut cleaned = Vec::with_capacity(keywords.len());
    for keyword in keywords {
        let stripped = keyword.trim();
        if !stripped.is_empty() {
            cleaned.push(stripped.to_owned());
        }
    }
    if cleaned.is_empty() {
        return Err(invalid(&format!("{path}.keywords")));
    }
    let Some(tier) = raw.tier else {
        return Err(invalid(&format!("{path}.tier")));
    };
    let tier = tier.trim();
    if tier.is_empty() {
        return Err(invalid(&format!("{path}.tier")));
    }
    Ok(KeywordTierRule {
        keywords: cleaned,
        tier: tier.to_owned(),
    })
}

// a bool arrives as a literal or an os.environ reference the proxy resolves before pydantic reads it
pub(super) fn resolve_flag<L>(
    raw: Option<RawFlag>,
    path: &str,
    lookup: &L,
) -> Result<Option<bool>, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        None => Ok(None),
        Some(RawFlag::Literal(flag)) => Ok(Some(flag)),
        Some(RawFlag::Text(text)) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?)
        }
    }
}

fn window<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(found) = resolve_number(value, path, lookup, parse_integer)? else {
        return Ok(DEFAULT_STALL_ESCALATION_WINDOW);
    };
    if found == 0 {
        return Err(invalid(path));
    }
    Ok(found)
}

fn repeat_threshold<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(found) = resolve_number(value, path, lookup, parse_integer)? else {
        return Ok(DEFAULT_STALL_ESCALATION_REPEAT_THRESHOLD);
    };
    if found < 2 {
        return Err(invalid(path));
    }
    Ok(found)
}

fn threshold<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(declared) = &value {
        reject_quoted(declared, path)?;
    }
    let Some(found) = resolve_number(value, path, lookup, parse_float)? else {
        return Ok(DEFAULT_MATCH_THRESHOLD);
    };
    if !found.is_finite() || found < 0.0_f64 || found > 1.0_f64 {
        return Err(invalid(path));
    }
    Ok(found)
}

// an os.environ/ reference resolves once here, and an empty variable counts as unset
fn optional_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let is_reference = text.starts_with(ENV_MARKER);
    let resolved = resolve_string(text, path, lookup)?;
    if is_reference && resolved.is_empty() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

// a declared number is not text; only an os.environ reference may be
fn reject_quoted(value: &RawNumber, path: &str) -> Result<(), ConfigError> {
    if let RawNumber::Text(text) = value
        && !text.starts_with(ENV_MARKER)
    {
        return Err(invalid(path));
    }
    Ok(())
}

fn invalid(path: &str) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    }
}
