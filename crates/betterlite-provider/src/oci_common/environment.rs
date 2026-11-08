use http::HeaderMap;
use http::header::{CONTENT_TYPE, HeaderValue, USER_AGENT};

use crate::oci_common::OciError;

pub const DEFAULT_LITELLM_VERSION: &str = "1.105.1";

#[derive(Debug, Clone)]
pub struct OciEnvironment {
    pub version: String,
}

impl Default for OciEnvironment {
    fn default() -> Self {
        Self {
            version: DEFAULT_LITELLM_VERSION.to_owned(),
        }
    }
}

pub fn validate_oci_environment(
    headers: &mut HeaderMap,
    environment: &OciEnvironment,
) -> Result<(), OciError> {
    headers
        .entry(CONTENT_TYPE)
        .or_insert(HeaderValue::from_static("application/json"));
    if !headers.contains_key(USER_AGENT) {
        let user_agent = HeaderValue::from_str(&format!("litellm/{}", environment.version))
            .map_err(|source| OciError {
                status: 500,
                message: format!("the OCI user-agent is not a valid header value: {source}"),
            })?;
        headers.insert(USER_AGENT, user_agent);
    }
    Ok(())
}
