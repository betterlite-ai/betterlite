use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

mod compare;
mod tests;

pub(crate) use self::compare::compare_documents;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NumericRule {
    pub pointer: String,
    // a YAML float arrives as f64, so the library visitor keeps the shortest decimal text
    #[serde(deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize")]
    pub absolute_tolerance: Decimal,
    pub why: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExpectedDifference {
    pub pointer: String,
    pub reference: Value,
    pub rust: Value,
    pub why: String,
    pub link: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Contracts {
    #[serde(default)]
    pub numeric: Vec<NumericRule>,
    #[serde(default)]
    pub expected_difference: Vec<ExpectedDifference>,
    // set by the runner, never read from a case file
    #[serde(skip)]
    pub self_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("compare.{kind} pointer \"{pointer}\" is not a JSON pointer")]
    Pointer { kind: &'static str, pointer: String },
    #[error("compare.{kind} pointer \"{pointer}\" is named more than once")]
    Duplicate { kind: &'static str, pointer: String },
    #[error("compare.{kind} pointer \"{pointer}\" overlaps \"{other}\"")]
    Overlap {
        kind: &'static str,
        pointer: String,
        other: String,
    },
    #[error("compare.numeric pointer \"{pointer}\" names {category}, which stays exact")]
    ExactOnly {
        pointer: String,
        category: &'static str,
    },
    #[error("compare.numeric pointer \"{pointer}\" has a negative absolute_tolerance")]
    NegativeTolerance { pointer: String },
    #[error("compare.{kind} pointer \"{pointer}\" needs a {field}")]
    MissingText {
        kind: &'static str,
        pointer: String,
        field: &'static str,
    },
    #[error("compare.expected_difference pointer \"{pointer}\" link must be empty or an https URL")]
    Link { pointer: String },
}

impl ContractError {
    fn pointer(&self) -> &str {
        match self {
            Self::Pointer { pointer, .. }
            | Self::Duplicate { pointer, .. }
            | Self::Overlap { pointer, .. }
            | Self::ExactOnly { pointer, .. }
            | Self::NegativeTolerance { pointer, .. }
            | Self::MissingText { pointer, .. }
            | Self::Link { pointer, .. } => pointer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mismatch {
    pub(crate) pointer: String,
    pub(crate) message: String,
}

pub fn validate(
    numeric: &[NumericRule],
    expected_difference: &[ExpectedDifference],
) -> Result<(), ContractError> {
    for rule in numeric {
        validate_pointer("numeric", &rule.pointer)?;
        if rule.absolute_tolerance < Decimal::ZERO {
            return Err(ContractError::NegativeTolerance {
                pointer: rule.pointer.clone(),
            });
        }
        if rule.why.trim().is_empty() {
            return Err(ContractError::MissingText {
                kind: "numeric",
                pointer: rule.pointer.clone(),
                field: "why",
            });
        }
        if let Some(category) = exact_only_category(&rule.pointer) {
            return Err(ContractError::ExactOnly {
                pointer: rule.pointer.clone(),
                category,
            });
        }
    }
    for rule in expected_difference {
        validate_pointer("expected_difference", &rule.pointer)?;
        if rule.why.trim().is_empty() {
            return Err(ContractError::MissingText {
                kind: "expected_difference",
                pointer: rule.pointer.clone(),
                field: "why",
            });
        }
        if !(rule.link.is_empty() || rule.link.starts_with("https://")) {
            return Err(ContractError::Link {
                pointer: rule.pointer.clone(),
            });
        }
    }
    check_unique_and_disjoint(numeric, expected_difference)
}

fn validate_pointer(kind: &'static str, pointer: &str) -> Result<(), ContractError> {
    if pointer_is_valid(pointer) {
        return Ok(());
    }
    Err(ContractError::Pointer {
        kind,
        pointer: pointer.to_owned(),
    })
}

fn pointer_is_valid(pointer: &str) -> bool {
    if !pointer.starts_with('/') {
        return false;
    }
    let mut characters = pointer.chars();
    while let Some(character) = characters.next() {
        if character == '~' && !matches!(characters.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

// a tolerance on any of these would hide a changed decision instead of a float diagnostic
const EXACT_ONLY: &[(&str, &str)] = &[
    ("allowed", "a permission outcome"),
    ("authorized", "a permission outcome"),
    ("code", "a status code"),
    ("cost", "spend"),
    ("count", "a count"),
    ("deployment", "the selected model"),
    ("denied", "a permission outcome"),
    ("model", "the selected model"),
    ("model_name", "the selected model"),
    ("permission", "a permission outcome"),
    ("permissions", "a permission outcome"),
    ("price", "spend"),
    ("provider_requests", "a provider request"),
    ("selected", "the selected model"),
    ("selection", "the selected model"),
    ("spend", "spend"),
    ("status", "a status code"),
    ("status_code", "a status code"),
    ("status_codes", "a status code"),
    ("tier", "a tier"),
    ("tier_name", "a tier"),
    ("total_tokens", "a count"),
    ("upstream", "a provider request"),
];

const EXACT_ONLY_SUFFIXES: &[(&str, &str)] = &[
    ("_cost", "spend"),
    ("_count", "a count"),
    ("_price", "spend"),
    ("_spend", "spend"),
    ("_status", "a status code"),
    ("_tokens", "a count"),
];

fn exact_only_category(pointer: &str) -> Option<&'static str> {
    for segment in pointer.split('/').skip(1) {
        let name = segment
            .replace("~1", "/")
            .replace("~0", "~")
            .to_ascii_lowercase();
        if let Some((_, category)) = EXACT_ONLY.iter().find(|(word, _)| *word == name.as_str()) {
            return Some(*category);
        }
        if let Some((_, category)) = EXACT_ONLY_SUFFIXES
            .iter()
            .find(|(suffix, _)| name.ends_with(*suffix))
        {
            return Some(*category);
        }
    }
    None
}

fn check_unique_and_disjoint(
    numeric: &[NumericRule],
    expected_difference: &[ExpectedDifference],
) -> Result<(), ContractError> {
    let mut seen: Vec<(&'static str, &str)> = Vec::new();
    for (kind, pointer) in numeric
        .iter()
        .map(|rule| ("numeric", rule.pointer.as_str()))
        .chain(
            expected_difference
                .iter()
                .map(|rule| ("expected_difference", rule.pointer.as_str())),
        )
    {
        for (_, other) in &seen {
            if *other == pointer {
                return Err(ContractError::Duplicate {
                    kind,
                    pointer: pointer.to_owned(),
                });
            }
            if overlaps(pointer, other) {
                return Err(ContractError::Overlap {
                    kind,
                    pointer: pointer.to_owned(),
                    other: (*other).to_owned(),
                });
            }
        }
        seen.push((kind, pointer));
    }
    Ok(())
}

fn overlaps(one: &str, other: &str) -> bool {
    (one.starts_with(other) && one.as_bytes().get(other.len()) == Some(&b'/'))
        || (other.starts_with(one) && other.as_bytes().get(one.len()) == Some(&b'/'))
}
