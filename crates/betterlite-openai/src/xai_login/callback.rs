use std::str;

use secrecy::SecretString;
use url::{Url, form_urlencoded};

pub const CALLBACK_PATH: &str = "/callback";
pub const MAX_TARGET_BYTES: usize = 0x0001_0000;
// a relative request target resolves under this base, so only a target with its own leading slash can name the callback
const BASE_URL: &str = "http://127.0.0.1/x/";

#[derive(Debug, Default)]
pub struct CallbackFields {
    pub code: Option<SecretString>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug)]
pub struct CallbackRequest {
    pub path: String,
    pub fields: CallbackFields,
}

#[derive(Debug, thiserror::Error)]
pub enum CallbackError {
    #[error("the callback request target exceeds the size bound")]
    TooLong,
    #[error("the callback request target is not a valid URL")]
    Malformed,
}

pub fn decode_xai_callback(target: &[u8]) -> Result<CallbackRequest, CallbackError> {
    if target.len() > MAX_TARGET_BYTES {
        return Err(CallbackError::TooLong);
    }
    let text = str::from_utf8(target).map_err(|_error| CallbackError::Malformed)?;
    let base = Url::parse(BASE_URL).map_err(|_error| CallbackError::Malformed)?;
    let url = Url::options()
        .base_url(Some(&base))
        .parse(text)
        .map_err(|_error| CallbackError::Malformed)?;
    let mut fields = CallbackFields::default();
    for (name, value) in form_urlencoded::parse(url.query().unwrap_or_default().as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match name.as_ref() {
            "code" if fields.code.is_none() => {
                fields.code = Some(SecretString::from(value.into_owned()));
            }
            "state" if fields.state.is_none() => fields.state = Some(value.into_owned()),
            "error" if fields.error.is_none() => fields.error = Some(value.into_owned()),
            "error_description" if fields.error_description.is_none() => {
                fields.error_description = Some(value.into_owned());
            }
            _ => {}
        }
    }
    Ok(CallbackRequest {
        path: url.path().to_owned(),
        fields,
    })
}
