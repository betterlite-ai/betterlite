use super::error::{DurationError, DurationWarning};

const WORD_ALIASES: [(&str, &str); 4] = [
    ("daily", "24h"),
    ("hourly", "1h"),
    ("monthly", "30d"),
    ("weekly", "7d"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurationSpec {
    pub amount: u64,
    pub unit: DurationUnit,
    pub warnings: Vec<DurationWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetDurationSpec {
    Known {
        amount: u64,
        unit: ResetUnit,
        warnings: Vec<DurationWarning>,
    },
    Unrecognized {
        warnings: Vec<DurationWarning>,
    },
    UnknownUnit {
        unit: String,
        warnings: Vec<DurationWarning>,
    },
}

pub fn parse_budget_duration(spec: &str) -> Result<DurationSpec, DurationError> {
    let text = normalize(spec);
    let (digits, rest) = split_digits(text);
    if digits.is_empty() {
        return Err(DurationError::InvalidFormat {
            spec: spec.to_owned(),
        });
    }
    let (unit, trailing) = budget_unit(rest);
    let Some(known_unit) = unit else {
        return Err(DurationError::UnsupportedUnit {
            spec: spec.to_owned(),
        });
    };
    let amount = parse_amount(digits, spec)?;
    let mut warnings = Vec::new();
    if !trailing.is_empty() {
        warnings.push(DurationWarning::TrailingInput {
            rest: trailing.to_owned(),
        });
    }
    Ok(DurationSpec {
        amount,
        unit: known_unit,
        warnings,
    })
}

pub fn parse_reset_duration(spec: &str) -> Result<ResetDurationSpec, DurationError> {
    let text = normalize(spec);
    let (digits, rest) = split_digits(text);
    let (letters, trailing) = split_letters(rest);
    if digits.is_empty() || letters.is_empty() {
        return Ok(ResetDurationSpec::Unrecognized {
            warnings: vec![DurationWarning::UnrecognizedResetDuration {
                spec: spec.to_owned(),
            }],
        });
    }
    let mut warnings = Vec::new();
    if !trailing.is_empty() {
        warnings.push(DurationWarning::TrailingInput {
            rest: trailing.to_owned(),
        });
    }
    let Some(unit) = reset_unit(letters) else {
        warnings.push(DurationWarning::UnknownResetUnit {
            unit: letters.to_owned(),
        });
        return Ok(ResetDurationSpec::UnknownUnit {
            unit: letters.to_owned(),
            warnings,
        });
    };
    let amount = parse_amount(digits, spec)?;
    Ok(ResetDurationSpec::Known {
        amount,
        unit,
        warnings,
    })
}

fn normalize(spec: &str) -> &str {
    let key = spec.trim().to_lowercase();
    WORD_ALIASES
        .iter()
        .find(|(word, _)| *word == key)
        .map_or(spec, |(_, alias)| *alias)
}

fn split_digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    text.split_at_checked(end).unwrap_or((text, ""))
}

fn split_letters(text: &str) -> (&str, &str) {
    let end = text
        .find(|character: char| !character.is_ascii_lowercase())
        .unwrap_or(text.len());
    text.split_at_checked(end).unwrap_or((text, ""))
}

fn budget_unit(text: &str) -> (Option<DurationUnit>, &str) {
    if let Some(trailing) = text.strip_prefix("mo") {
        return (Some(DurationUnit::Month), trailing);
    }
    let Some(first) = text.chars().next() else {
        return (None, text);
    };
    let unit = match first {
        's' => Some(DurationUnit::Second),
        'm' => Some(DurationUnit::Minute),
        'h' => Some(DurationUnit::Hour),
        'd' => Some(DurationUnit::Day),
        'w' => Some(DurationUnit::Week),
        _ => None,
    };
    unit.map_or((None, text), |known| {
        (Some(known), text.get(1..).unwrap_or_default())
    })
}

fn reset_unit(letters: &str) -> Option<ResetUnit> {
    match letters {
        "s" => Some(ResetUnit::Second),
        "m" => Some(ResetUnit::Minute),
        "h" => Some(ResetUnit::Hour),
        "d" => Some(ResetUnit::Day),
        "w" => Some(ResetUnit::Week),
        "mo" => Some(ResetUnit::Month),
        _ => None,
    }
}

fn parse_amount(digits: &str, spec: &str) -> Result<u64, DurationError> {
    digits
        .parse::<u64>()
        .map_err(|source| DurationError::QuantityOverflow {
            spec: spec.to_owned(),
            source,
        })
}
