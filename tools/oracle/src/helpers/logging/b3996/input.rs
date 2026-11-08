use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_logging::gcs_mock::request::{GcsReadRequest, GcsRequest, GcsWriteRequest};
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;

use super::failure;
use crate::helpers::HelperError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) original: Option<OriginalResponse>,
    #[serde(default)]
    pub(crate) requests: Option<Vec<RequestInput>>,
    #[serde(default)]
    pub(crate) auth: Option<AuthInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalResponse {
    pub(crate) status: u16,
    pub(crate) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthInput {
    #[serde(default = "default_installs")]
    pub(crate) installs: u32,
}

const fn default_installs() -> u32 {
    1
}

#[derive(Clone, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub(crate) enum RequestInput {
    Get(ReadInput),
    Post(WriteInput),
    Delete(WriteInput),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadInput {
    url: String,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    follow_redirects: Option<bool>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WriteInput {
    url: String,
    #[serde(default)]
    data: Option<Json>,
    #[serde(default)]
    json: Option<Json>,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    content: Option<String>,
}

impl RequestInput {
    pub(crate) fn into_request(self) -> Result<GcsRequest, Box<HelperError>> {
        match self {
            Self::Get(input) => Ok(GcsRequest::Get(GcsReadRequest {
                url: input.url,
                params: input.params,
                headers: header_map(input.headers)?,
                follow_redirects: input.follow_redirects,
            })),
            Self::Post(input) => Ok(GcsRequest::Post(input.into_write()?)),
            Self::Delete(input) => Ok(GcsRequest::Delete(input.into_write()?)),
        }
    }
}

impl WriteInput {
    fn into_write(self) -> Result<GcsWriteRequest, Box<HelperError>> {
        Ok(GcsWriteRequest {
            url: self.url,
            data: self.data,
            json: self.json,
            params: self.params,
            headers: header_map(self.headers)?,
            timeout: self.timeout_ms.map(Duration::from_millis),
            stream: self.stream,
            content: self.content.map(String::into_bytes),
        })
    }
}

fn header_map(headers: BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (raw_name, raw_value) in headers {
        let name = HeaderName::from_bytes(raw_name.as_bytes())
            .map_err(|error| failure("ValueError", error))?;
        let value =
            HeaderValue::from_str(&raw_value).map_err(|error| failure("ValueError", error))?;
        map.insert(name, value);
    }
    Ok(map)
}
