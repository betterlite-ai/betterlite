use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, parse_integer, resolve_number, resolve_string};

use super::{
    Config, DEFAULT_HOUSEKEEPING, DEFAULT_MATCH_THRESHOLD, DEFAULT_SEMANTIC_MATCHING,
    DEFAULT_STALL_ENABLED, DEFAULT_STALL_REPEAT, DEFAULT_STALL_WINDOW, KeywordMatching,
    KeywordTierRule, PATH_ROOT, RawFlag, RawKeywordTierRule, RawRouterConfig,
};

const ENV_MARKER: &str = "os.environ/";

struct Semantic {
    matching: KeywordMatching,
    model: Option<String>,
    threshold: f64,
}

// the eight complexity_router_config keys of one deployment, normalized the way the section reads them
pub(super) fn router_config<L>(
    raw: RawRouterConfig,
    index: usize,
    lookup: &L,
) -> Result<Config, ConfigError>
where
    L: EnvironmentLookup,
{
    let prefix = format!("model_list.{index}.{PATH_ROOT}");
    let keyword_tier_rules = keyword_rules(raw.keyword_tier_rules, &prefix)?;
    let (stall_enabled, stall_window, stall_repeat) = stall(
        raw.stall_escalation_enabled,
        raw.stall_escalation_window,
        raw.stall_repeat_threshold,
        &prefix,
        lookup,
    )?;
    let semantic_settings = semantic(
        raw.semantic_keyword_matching,
        raw.embedding_model,
        raw.match_threshold,
        &prefix,
        lookup,
    )?;
    let housekeeping_cheapest = flag(
        raw.housekeeping_cheapest,
        &format!("{prefix}.route_housekeeping_to_cheapest_tier"),
        lookup,
        DEFAULT_HOUSEKEEPING,
    )?;
    Ok(Config {
        keyword_tier_rules,
        stall_escalation_enabled: stall_enabled,
        stall_escalation_window: stall_window,
        stall_repeat_threshold: stall_repeat,
        keyword_matching: semantic_settings.matching,
        embedding_model: semantic_settings.model,
        match_threshold: semantic_settings.threshold,
        housekeeping_cheapest,
    })
}

fn stall<L>(
    enabled_flag: Option<RawFlag>,
    window_value: Option<RawNumber>,
    repeat_value: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<(bool, u32, u32), ConfigError>
where
    L: EnvironmentLookup,
{
    let window = integer(
        window_value,
        &format!("{prefix}.stall_escalation_window"),
        DEFAULT_STALL_WINDOW,
        1,
        lookup,
    )?;
    let repeat = integer(
        repeat_value,
        &format!("{prefix}.stall_escalation_repeat_threshold"),
        DEFAULT_STALL_REPEAT,
        2,
        lookup,
    )?;
    let enabled = flag(
        enabled_flag,
        &format!("{prefix}.stall_escalation_enabled"),
        lookup,
        DEFAULT_STALL_ENABLED,
    )?;
    Ok((enabled, window, repeat))
}

fn semantic<L>(
    declared: Option<RawFlag>,
    model: Option<String>,
    threshold_value: Option<RawNumber>,
    prefix: &str,
    lookup: &L,
) -> Result<Semantic, ConfigError>
where
    L: EnvironmentLookup,
{
    let enabled = flag(
        declared,
        &format!("{prefix}.semantic_keyword_matching"),
        lookup,
        DEFAULT_SEMANTIC_MATCHING,
    )?;
    let embedding = optional_text(model, &format!("{prefix}.embedding_model"), lookup)?;
    let threshold = ratio(
        threshold_value,
        &format!("{prefix}.match_threshold"),
        lookup,
    )?;
    Ok(Semantic {
        matching: if enabled {
            KeywordMatching::Semantic
        } else {
            KeywordMatching::Lexical
        },
        model: embedding,
        threshold,
    })
}

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
    let Some(decorated) = raw.tier else {
        return Err(invalid(&format!("{path}.tier")));
    };
    let tier = decorated.trim();
    if tier.is_empty() {
        return Err(invalid(&format!("{path}.tier")));
    }
    Ok(KeywordTierRule {
        keywords: cleaned,
        tier: tier.to_owned(),
    })
}

// a bool arrives as a literal or an os.environ reference the proxy resolves before pydantic reads it
fn flag<L>(
    declared: Option<RawFlag>,
    path: &str,
    lookup: &L,
    default: bool,
) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = declared else {
        return Ok(default);
    };
    match raw {
        RawFlag::Literal(found) => Ok(found),
        RawFlag::Text(text) if !text.starts_with(ENV_MARKER) => Err(invalid(path)),
        RawFlag::Text(text) => {
            let name = text.strip_prefix(ENV_MARKER).unwrap_or(path).to_owned();
            let resolved = resolve_string(text, path, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(default))
        }
    }
}

// a whole number at or above the minimum; an absent key takes the default
fn integer<L>(
    value: Option<RawNumber>,
    path: &str,
    default: u32,
    minimum: u32,
    lookup: &L,
) -> Result<u32, ConfigError>
where
    L: EnvironmentLookup,
{
    match resolve_number(value, path, lookup, parse_integer)? {
        None => Ok(default),
        Some(found) if found < minimum => Err(invalid(path)),
        Some(found) => Ok(found),
    }
}

fn ratio<L>(value: Option<RawNumber>, path: &str, lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(declared) = &value {
        reject_quoted(declared, path)?;
    }
    let Some(found) = resolve_number(value, path, lookup, parse_float)? else {
        return Ok(DEFAULT_MATCH_THRESHOLD);
    };
    if found.is_finite() && (0.0_f64..=1.0_f64).contains(&found) {
        return Ok(found);
    }
    Err(invalid(path))
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
