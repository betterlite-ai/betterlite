use serde::Deserialize;

pub mod capture;
pub(crate) mod request;

pub(crate) const MAX_FIXTURE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_PARTS: usize = 128;
pub(crate) const MAX_CAPTURED_BODY_BYTES: usize = 16 * 1024 * 1024;

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
    #[serde(default)]
    pub(crate) body_kind: Option<crate::case::BodyKind>,
    pub(crate) fixture: String,
    pub(crate) content_type: String,
}

mod tests;
