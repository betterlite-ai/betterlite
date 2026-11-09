use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;

use crate::image_models::params::{Field, Params};

pub type Parsed<T> = Result<T, Vec<String>>;

pub trait Choice: Copy + 'static {
    const EXPECTED: &'static str;
    const NAMES: &'static [(&'static str, Self)];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    Text,
    Image,
    Video,
    Audio,
    TextImage,
    MultiInput,
}

impl InputType {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::TextImage => "text_image",
            Self::MultiInput => "multi_input",
        }
    }
}

impl Choice for InputType {
    const EXPECTED: &'static str =
        "Input should be 'text', 'image', 'video', 'audio', 'text_image' or 'multi_input'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("text", Self::Text),
        ("image", Self::Image),
        ("video", Self::Video),
        ("audio", Self::Audio),
        ("text_image", Self::TextImage),
        ("multi_input", Self::MultiInput),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingOption {
    Visual,
    Audio,
    Transcription,
}

impl Choice for EmbeddingOption {
    const EXPECTED: &'static str = "Input should be 'visual', 'audio' or 'transcription'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("visual", Self::Visual),
        ("audio", Self::Audio),
        ("transcription", Self::Transcription),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingType {
    SeparateEmbedding,
    FusedEmbedding,
}

impl Choice for EmbeddingType {
    const EXPECTED: &'static str = "Input should be 'separate_embedding' or 'fused_embedding'";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("separate_embedding", Self::SeparateEmbedding),
        ("fused_embedding", Self::FusedEmbedding),
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingScope {
    Clip,
    Asset,
}

impl Choice for EmbeddingScope {
    const EXPECTED: &'static str = "Input should be 'clip' or 'asset'";
    const NAMES: &'static [(&'static str, Self)] = &[("clip", Self::Clip), ("asset", Self::Asset)];
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

pub fn text(value: &Field, location: &str) -> Parsed<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| vec![format!("{location}: Input should be a valid string")])
}

// Pydantic lax mode: bools count as 0 and 1 and numeric strings are read as numbers
pub fn number(value: &Field, location: &str) -> Parsed<Number> {
    if let Field::Int(whole) = value {
        return Ok(Number::from(*whole));
    }
    if let Field::Bool(flag) = value {
        return Ok(Number::from(i64::from(*flag)));
    }
    if let Field::Float(float) = value
        && let Some(finite) = Number::from_f64(*float)
    {
        return Ok(finite);
    }
    if let Field::Text(text) = value
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

pub fn integer(value: &Field, location: &str) -> Parsed<Number> {
    let invalid = || vec![format!("{location}: Input should be a valid integer")];
    if matches!(value, Field::Text(text) if text.contains(['e', 'E'])) {
        return Err(invalid());
    }
    let parsed = number(value, location).map_err(|_unparsed| invalid())?;
    let Some(float) = parsed.as_f64().filter(|_| parsed.is_f64()) else {
        return Ok(parsed);
    };
    // A whole float prints without a fraction, so it reads back as an integer
    let rendered = float.to_string();
    if rendered.contains('.') {
        return Err(vec![format!(
            "{location}: Input should be a valid integer, got a number with a fractional part"
        )]);
    }
    let whole = rendered.parse::<Number>().ok();
    whole
        .filter(|candidate| !candidate.is_f64())
        .ok_or_else(invalid)
}

pub fn choice<T>(value: &Field, location: &str) -> Parsed<T>
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
    value: &Field,
    location: &str,
    parse: fn(&Field, &str) -> Parsed<T>,
) -> Parsed<Vec<T>> {
    let Field::List(items) = value else {
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

pub fn media_sources(entries: &Params, location: &str) -> Parsed<IndexMap<String, String>> {
    let named = entries
        .iter()
        .map(|(name, media)| {
            text(media, &format!("{location}.{name}")).map(|source| (name.clone(), source))
        })
        .collect();
    collect(named).map(IndexMap::from_iter)
}

pub fn segmentation(value: &Field, location: &str) -> Parsed<Segmentation> {
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
    value: &Field,
    location: &str,
) -> Parsed<Number> {
    let prefix = format!("{location}.{}", branch.label);
    let Field::Mapping(object) = value else {
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

fn branch_duration(branch: &SegmentationBranch, object: &Params, prefix: &str) -> Parsed<Number> {
    let nested = format!("{prefix}.{}", branch.key);
    match object.get(branch.key) {
        None => Err(vec![format!("{nested}: Field required")]),
        Some(Field::Mapping(inner)) => {
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
