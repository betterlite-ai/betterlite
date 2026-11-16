use serde::Deserialize;

use crate::case::BodyKind;

pub mod capture;
pub mod error;
pub mod request;

pub struct OracleLimits {
    pub fixture_bytes: usize,
    pub parts: usize,
    pub captured_body_bytes: usize,
}

pub const LIMITS: OracleLimits = OracleLimits {
    fixture_bytes: 16 * 1024 * 1024,
    parts: 128,
    captured_body_bytes: 16 * 1024 * 1024,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartSpec {
    // an absent name reaches the empty-name diagnostic instead of serde's missing-field one
    #[serde(default)]
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub text: Option<String>,
    pub fixture: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultipartSpec {
    pub parts: Vec<PartSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BytesSpec {
    pub fixture: String,
    pub content_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BytesAnswer {
    pub body_kind: BodyKind,
    pub fixture: String,
    pub content_type: String,
}
