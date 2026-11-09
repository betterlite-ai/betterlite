use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::integer_text::whole_digits;
use super::options::Choice;

pub type Parsed<T> = Result<T, Vec<String>>;

// Numbers stay serde_json::Number so decoding does not depend on the arbitrary_precision feature
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Given {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Mapping(Entries),
}

pub type Entries = IndexMap<String, Given>;

impl Given {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Text(text) = self {
            Some(text)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum Segmentation {
    Fixed { fixed: FixedDuration },
    Dynamic { dynamic: DynamicDuration },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixedDuration {
    pub duration_sec: Number,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicDuration {
    pub min_duration_sec: Number,
}

struct SegmentationBranch {
    label: &'static str,
    method: &'static str,
    key: &'static str,
    leaf: &'static str,
    build: fn(Number) -> Segmentation,
}

const SEGMENTATION_BRANCHES: [SegmentationBranch; 2] = [
    SegmentationBranch {
        label: "TwelveLabsMarengo3FixedSegmentation",
        method: "fixed",
        key: "fixed",
        leaf: "durationSec",
        build: |duration_sec| Segmentation::Fixed {
            fixed: FixedDuration { duration_sec },
        },
    },
    SegmentationBranch {
        label: "TwelveLabsMarengo3DynamicSegmentation",
        method: "dynamic",
        key: "dynamic",
        leaf: "minDurationSec",
        build: |min_duration_sec| Segmentation::Dynamic {
            dynamic: DynamicDuration { min_duration_sec },
        },
    },
];

pub fn text(value: &Given, location: &str) -> Parsed<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| vec![format!("{location}: Input should be a valid string")])
}

// Pydantic lax mode: bools count as 0 and 1 and numeric strings are read as numbers
pub fn number(value: &Given, location: &str) -> Parsed<Number> {
    if let Given::Number(parsed) = value {
        return Ok(parsed.clone());
    }
    if let Given::Bool(flag) = value {
        return Ok(Number::from(i64::from(*flag)));
    }
    if let Given::Text(text) = value
        && let Some(parsed) = parse_numeric_text(text)
    {
        return Ok(parsed);
    }
    Err(vec![format!("{location}: Input should be a valid number")])
}

fn parse_numeric_text(text: &str) -> Option<Number> {
    let trimmed = text.trim();
    let whole = trimmed.parse::<i64>().ok().map(Number::from);
    whole.or_else(|| trimmed.parse::<f64>().ok().and_then(Number::from_f64))
}

pub fn integer(value: &Given, location: &str) -> Parsed<Number> {
    let invalid = || vec![format!("{location}: Input should be a valid integer")];
    if let Given::Text(text) = value {
        return whole_digits(text.trim(), true).ok_or_else(invalid);
    }
    let parsed = number(value, location).map_err(|_unparsed| invalid())?;
    if let Some(whole) = whole_digits(&parsed.to_string(), false) {
        return Ok(whole);
    }
    let float = parsed.as_f64().ok_or_else(invalid)?;
    // A whole float prints without a fraction, so it reads back as an integer
    let rendered = float.to_string();
    if rendered.contains('.') {
        return Err(vec![format!(
            "{location}: Input should be a valid integer, got a number with a fractional part"
        )]);
    }
    rendered
        .parse::<i64>()
        .map(Number::from)
        .map_err(|_unparsed| invalid())
}

pub fn choice<T>(value: &Given, location: &str) -> Parsed<T>
where
    T: Choice,
{
    let given = value.as_str().unwrap_or_default();
    for (name, selected) in T::NAMES {
        if *name == given {
            return Ok(*selected);
        }
    }
    Err(vec![format!("{location}: {}", T::EXPECTED)])
}

fn collect<T>(items: Vec<Parsed<T>>) -> Parsed<Vec<T>> {
    let mut parsed = Vec::with_capacity(items.len());
    let mut issues = Vec::new();
    for item in items {
        match item {
            Ok(value) => parsed.push(value),
            Err(lines) => issues.extend(lines),
        }
    }
    if issues.is_empty() {
        Ok(parsed)
    } else {
        Err(issues)
    }
}

pub fn list<T>(
    value: &Given,
    location: &str,
    parse: fn(&Given, &str) -> Parsed<T>,
) -> Parsed<Vec<T>> {
    let Given::List(items) = value else {
        return Err(vec![format!("{location}: Input should be a valid tuple")]);
    };
    collect(
        items
            .iter()
            .enumerate()
            .map(|(index, item)| parse(item, &format!("{location}.{index}")))
            .collect(),
    )
}

pub fn media_sources(entries: &Entries, location: &str) -> Parsed<IndexMap<String, String>> {
    let named = entries
        .iter()
        .map(|(name, media)| {
            text(media, &format!("{location}.{name}")).map(|source| (name.clone(), source))
        })
        .collect();
    collect(named).map(IndexMap::from_iter)
}

pub fn segmentation(value: &Given, location: &str) -> Parsed<Segmentation> {
    let mut issues = Vec::new();
    for branch in SEGMENTATION_BRANCHES {
        match segmentation_branch(&branch, value, location) {
            Ok(duration) => return Ok((branch.build)(duration)),
            Err(lines) => issues.extend(lines),
        }
    }
    Err(issues)
}

fn segmentation_branch(
    branch: &SegmentationBranch,
    value: &Given,
    location: &str,
) -> Parsed<Number> {
    let prefix = format!("{location}.{}", branch.label);
    let Given::Mapping(object) = value else {
        return Err(vec![format!(
            "{prefix}: Input should be a valid dictionary"
        )]);
    };
    let mut issues = Vec::new();
    match object.get("method") {
        Some(method) if method.as_str() == Some(branch.method) => {}
        Some(_) => issues.push(format!(
            "{prefix}.method: Input should be '{}'",
            branch.method
        )),
        None => issues.push(format!("{prefix}.method: Field required")),
    }
    match branch_duration(branch, object, &prefix) {
        Ok(duration) if issues.is_empty() => Ok(duration),
        Ok(_) => Err(issues),
        Err(lines) => {
            issues.extend(lines);
            Err(issues)
        }
    }
}

fn branch_duration(branch: &SegmentationBranch, object: &Entries, prefix: &str) -> Parsed<Number> {
    let nested = format!("{prefix}.{}", branch.key);
    match object.get(branch.key) {
        None => Err(vec![format!("{nested}: Field required")]),
        Some(Given::Mapping(inner)) => {
            let leaf = format!("{nested}.{}", branch.leaf);
            inner.get(branch.leaf).map_or_else(
                || Err(vec![format!("{leaf}: Field required")]),
                |duration| integer(duration, &leaf),
            )
        }
        Some(_) => Err(vec![format!(
            "{nested}: Input should be a valid dictionary"
        )]),
    }
}
