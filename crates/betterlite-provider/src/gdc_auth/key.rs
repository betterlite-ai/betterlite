use std::str::from_utf8;

use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

use crate::gdc_auth::error::GdcAuthError;

pub const MAX_KEY_BYTES: usize = 0x0001_0000;
const GDCH_TYPE: &str = "gdch_service_account";
const FORMAT_VERSION: &str = "1";
const RECURSION_LIMIT_MESSAGE: &str = "recursion limit exceeded";
const NONSTANDARD_JSON_LITERALS: [&str; 3] = ["NaN", "Infinity", "-Infinity"];

#[rustfmt::skip]
#[derive(Debug)]
pub enum GdcKey { Bearer(SecretString), ServiceAccount(GdchDescriptor) }
#[rustfmt::skip]
#[derive(Debug, Clone)]
pub struct GdchDescriptor { pub private_key_id: String, pub private_key: SecretString, pub name: String, pub project: String, pub token_uri: String, pub ca_cert_path: Option<String> }
#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("the credential is larger than {limit} bytes")] TooLarge { limit: usize },
    #[error("the credential is not UTF-8")] NotUtf8,
    #[error("the credential JSON is not a GDCH service account object")] Shape,
    #[error("the credential does not use the supported format version")] Version,
}
#[rustfmt::skip]
#[derive(Deserialize)]
struct RawCredential {
    #[serde(rename = "type")] kind: String, format_version: String, private_key_id: String, private_key: SecretString,
    name: String, project: String, token_uri: String, #[serde(default)] ca_cert_path: Option<String>,
}

#[rustfmt::skip]
pub fn decode_gdc_key(bytes: &[u8], model: &str) -> Result<GdcKey, GdcAuthError> {
    let decode = |source| GdcAuthError::Key { model: model.to_owned(), source };
    if bytes.len() > MAX_KEY_BYTES {
        return Err(decode(KeyError::TooLarge { limit: MAX_KEY_BYTES }));
    }
    let text = from_utf8(bytes).map_err(|_error| decode(KeyError::NotUtf8))?;
    if let Err(error) = serde_json::from_str::<IgnoredAny>(text) {
        // json.loads parses deeper nesting than serde_json's 128 levels, so valid JSON that deep is not a bearer token
        if error.to_string().starts_with(RECURSION_LIMIT_MESSAGE) {
            return Err(decode(KeyError::Shape));
        }
        // CPython's json.loads accepts these three bare literals, so they reach the credential-shape check
        if NONSTANDARD_JSON_LITERALS.contains(&text.trim()) {
            return Err(decode(KeyError::Shape));
        }
        return Ok(GdcKey::Bearer(SecretString::from(text.to_owned())));
    }
    let raw = serde_json::from_str::<RawCredential>(text).map_err(|_error| decode(KeyError::Shape))?;
    if raw.kind != GDCH_TYPE {
        return Err(decode(KeyError::Shape));
    }
    if raw.format_version != FORMAT_VERSION {
        return Err(decode(KeyError::Version));
    }
    Ok(GdcKey::ServiceAccount(GdchDescriptor { private_key_id: raw.private_key_id, private_key: raw.private_key, name: raw.name, project: raw.project, token_uri: raw.token_uri, ca_cert_path: raw.ca_cert_path }))
}
