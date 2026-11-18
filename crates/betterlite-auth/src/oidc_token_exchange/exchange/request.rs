use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::form_urlencoded;

use super::{FORM_CONTENT_TYPE, OidcTokenExchangeError, TokenExchangeRequest};

const JSON_ACCEPT: &str = "application/json";

pub(super) fn token_form(request: &TokenExchangeRequest) -> String {
    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("grant_type", "authorization_code");
    form.append_pair("code", &request.authorization_code);
    form.append_pair("code_verifier", request.code_verifier.expose_secret());
    if let Some(redirect_url) = request
        .redirect_url
        .as_deref()
        .filter(|url| !url.is_empty())
    {
        form.append_pair("redirect_uri", redirect_url);
    }
    match nonempty_secret(request) {
        Some(secret) if request.include_client_id => {
            form.append_pair("client_id", &request.client_id);
            form.append_pair("client_secret", secret.expose_secret());
        }
        Some(_secret) => {}
        None => {
            form.append_pair("client_id", &request.client_id);
        }
    }
    // LiteLLM encodes with urllib's quote_plus, which leaves `~` alone and escapes `*`
    form.finish().replace('*', "%2A").replace("%7E", "~")
}

pub(super) fn token_headers(
    request: &TokenExchangeRequest,
) -> Result<HeaderMap, OidcTokenExchangeError> {
    let mut headers = additional_headers(&request.additional_headers)?;
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE));
    headers.insert(ACCEPT, HeaderValue::from_static(JSON_ACCEPT));
    if !request.include_client_id
        && let Some(secret) = nonempty_secret(request)
    {
        let credentials =
            STANDARD.encode(format!("{}:{}", request.client_id, secret.expose_secret()));
        let value = authorization_header(format!("Basic {credentials}"))?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

pub(super) fn authorization_header(value: String) -> Result<HeaderValue, OidcTokenExchangeError> {
    HeaderValue::try_from(value).map_err(|source| OidcTokenExchangeError::HeaderValue {
        name: AUTHORIZATION.as_str().to_owned(),
        source,
    })
}

pub(super) fn additional_headers(
    additional: &[(String, String)],
) -> Result<HeaderMap, OidcTokenExchangeError> {
    let mut headers = HeaderMap::new();
    for (name, value) in additional {
        let header_name = HeaderName::try_from(name.as_str()).map_err(|source| {
            OidcTokenExchangeError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value = HeaderValue::try_from(value.as_str()).map_err(|source| {
            OidcTokenExchangeError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn nonempty_secret(request: &TokenExchangeRequest) -> Option<&SecretString> {
    request
        .client_secret
        .as_ref()
        .filter(|secret| !secret.expose_secret().is_empty())
}
