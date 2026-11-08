use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use http::HeaderMap;

pub const GCS_HOST: &str = "storage.googleapis.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "UPPERCASE")]
pub enum GcsMethod {
    Get,
    Post,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsReadRequest {
    pub url: String,
    pub params: BTreeMap<String, String>,
    pub headers: HeaderMap,
    pub follow_redirects: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcsWriteRequest {
    pub url: String,
    pub data: Option<Json>,
    pub json: Option<Json>,
    pub params: BTreeMap<String, String>,
    pub headers: HeaderMap,
    pub timeout: Option<Duration>,
    pub stream: bool,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GcsRequest {
    Get(GcsReadRequest),
    Post(GcsWriteRequest),
    Delete(GcsWriteRequest),
}

impl GcsRequest {
    #[must_use]
    pub const fn method(&self) -> GcsMethod {
        match self {
            Self::Get(_) => GcsMethod::Get,
            Self::Post(_) => GcsMethod::Post,
            Self::Delete(_) => GcsMethod::Delete,
        }
    }

    #[must_use]
    pub fn url(&self) -> &str {
        match self {
            Self::Get(request) => &request.url,
            Self::Post(request) | Self::Delete(request) => &request.url,
        }
    }

    #[must_use]
    pub fn targets_gcs(&self) -> bool {
        match self {
            Self::Get(_) | Self::Delete(_) => self.url().contains(GCS_HOST),
            Self::Post(_) => self.url().to_ascii_lowercase().contains(GCS_HOST),
        }
    }
}
