use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use secrecy::SecretString;
use url::Url;

use crate::url_policy::error::UrlPolicyError;

pub struct ConfiguredBase {
    pub url: Url,
    pub authorization: Option<SecretString>,
}

// HTTPX sends URL userinfo as the Authorization header, and it beats an api_key bearer
fn take_basic_authorization(
    url: &mut Url,
    key: &str,
) -> Result<Option<SecretString>, UrlPolicyError> {
    let username = url.username().to_owned();
    let password = url.password().map(str::to_owned);
    if username.is_empty() && password.is_none() {
        return Ok(None);
    }
    let decoded_user = percent_decode_str(&username)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_source| UrlPolicyError::InvalidUserinfo {
            key: key.to_owned(),
        })?;
    let decoded_password = match password {
        Some(value) => percent_decode_str(&value)
            .decode_utf8()
            .map(std::borrow::Cow::into_owned)
            .map_err(|_source| UrlPolicyError::InvalidUserinfo {
                key: key.to_owned(),
            })?,
        None => String::new(),
    };
    url.set_password(None)
        .map_err(|()| UrlPolicyError::InvalidUserinfo {
            key: key.to_owned(),
        })?;
    url.set_username("")
        .map_err(|()| UrlPolicyError::InvalidUserinfo {
            key: key.to_owned(),
        })?;
    let userpass = format!("{decoded_user}:{decoded_password}");
    Ok(Some(SecretString::from(format!(
        "Basic {}",
        STANDARD.encode(userpass)
    ))))
}

impl ConfiguredBase {
    pub fn parse(mut url: Url, key: &str) -> Result<Self, UrlPolicyError> {
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(UrlPolicyError::UnsupportedScheme {
                key: key.to_owned(),
                scheme: url.scheme().to_owned(),
            });
        }
        let authorization = take_basic_authorization(&mut url, key)?;
        Ok(Self { url, authorization })
    }
}
