use betterlite_config::sections::complexity_config as section;

use crate::complexity_config::checks::Checks;
use crate::complexity_config::error::ComplexityConfigError;

pub const MAX_CLASSIFICATION_PROMPT_CHARS: usize = 2000;
pub const MAX_CLASSIFICATION_EXAMPLES_CHARS: usize = 4000;

// the six keys and the three text settings they validate, after LiteLLM's field normalizers
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Normalized {
    pub escalation_keywords: Option<Vec<String>>,
    pub plan_mode_min_tier: Option<String>,
    pub plan_mode_patterns: Option<Vec<String>>,
    pub housekeeping_patterns: Option<Vec<String>>,
    pub heuristic_first_max_tier: Option<String>,
    pub hybrid_boundary_margin: Option<f64>,
    pub fallback_tier: Option<String>,
    pub classification_prompt: Option<String>,
    pub classification_examples: Option<String>,
}

pub fn apply(
    settings: &section::Config,
    checks: &Checks<'_>,
) -> Result<Normalized, ComplexityConfigError> {
    let normalized = Normalized {
        escalation_keywords: settings
            .escalation_keywords
            .as_deref()
            .map(stripped_keywords),
        plan_mode_min_tier: settings.plan_mode_min_tier.as_deref().map(strip),
        plan_mode_patterns: settings
            .plan_mode_patterns
            .as_deref()
            .map(stripped_keywords),
        housekeeping_patterns: settings
            .housekeeping_patterns
            .as_deref()
            .map(stripped_keywords),
        heuristic_first_max_tier: settings.heuristic_first_max_tier.as_deref().map(strip),
        hybrid_boundary_margin: settings.hybrid_boundary_margin,
        fallback_tier: blank_rejecting_text(checks.fallback_tier, "fallback_tier")?,
        classification_prompt: operator_section(
            checks.classification_prompt,
            "classification_prompt",
            MAX_CLASSIFICATION_PROMPT_CHARS,
        )?,
        classification_examples: operator_section(
            checks.classification_examples,
            "classification_examples",
            MAX_CLASSIFICATION_EXAMPLES_CHARS,
        )?,
    };
    if let Some(margin) = normalized.hybrid_boundary_margin
        && !(0.0_f64..=1.0_f64).contains(&margin)
    {
        return Err(ComplexityConfigError::HybridMarginOutOfRange { value: margin });
    }
    Ok(normalized)
}

// Python's str.strip whitespace includes the four information separators Rust's is_whitespace leaves out
const PYTHON_SEPARATORS: [char; 4] = ['\u{1C}', '\u{1D}', '\u{1E}', '\u{1F}'];

fn strip(text: &str) -> String {
    text.trim_matches(|character: char| {
        character.is_whitespace() || PYTHON_SEPARATORS.contains(&character)
    })
    .to_owned()
}

fn stripped_keywords(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| strip(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn blank_rejecting_text(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, ComplexityConfigError> {
    let Some(text) = value else {
        return Ok(None);
    };
    let stripped = strip(text);
    if stripped.is_empty() {
        return Err(ComplexityConfigError::BlankText { field });
    }
    Ok(Some(stripped))
}

fn operator_section(
    value: Option<&str>,
    field: &'static str,
    cap: usize,
) -> Result<Option<String>, ComplexityConfigError> {
    let Some(text) = blank_rejecting_text(value, field)? else {
        return Ok(None);
    };
    if text.chars().count() > cap {
        return Err(ComplexityConfigError::TextTooLong { field, max: cap });
    }
    Ok(Some(text))
}
