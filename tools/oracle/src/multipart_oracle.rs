use serde::Deserialize;

use crate::case::BodyKind;

pub mod capture;
pub mod error;
pub mod request;

pub(crate) struct OracleLimits {
    pub(crate) fixture_bytes: usize,
    pub(crate) parts: usize,
    pub(crate) captured_body_bytes: usize,
}

pub(crate) const LIMITS: OracleLimits = OracleLimits {
    fixture_bytes: 16 * 1024 * 1024,
    parts: 128,
    captured_body_bytes: 16 * 1024 * 1024,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub(crate) name: String,
    pub(crate) filename: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartSpec {
    // an absent name reaches the empty-name diagnostic instead of serde's missing-field one
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) filename: Option<String>,
    pub(crate) content_type: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) fixture: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MultipartSpec {
    pub(crate) parts: Vec<PartSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BytesSpec {
    pub(crate) fixture: String,
    pub(crate) content_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BytesAnswer {
    pub(crate) body_kind: BodyKind,
    pub(crate) fixture: String,
    pub(crate) content_type: String,
}

mod tests;
