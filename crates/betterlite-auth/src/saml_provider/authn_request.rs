use std::io::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use quick_xml::escape::escape;
use sha1::{Digest as _, Sha1};
use uuid::Uuid;

use super::error::SamlProviderError;
use super::settings::{HTTP_POST_BINDING, MergedSettings};

const REQUEST_ID_PREFIX: &str = "ONELOGIN_";

// python's quote_plus safe set: letters, digits and "_.-~", with a space as "+"
const FORM_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[must_use]
pub fn generate_request_id() -> String {
    let entropy = Uuid::new_v4().simple().to_string();
    let digest = hex::encode(Sha1::digest(entropy.as_bytes()));
    format!("{REQUEST_ID_PREFIX}{digest}")
}

pub fn authn_request_xml(
    settings: &MergedSettings,
    request_id: &str,
    now: i64,
) -> Result<String, SamlProviderError> {
    let issue_instant = super::timestamp(now).ok_or(SamlProviderError::ClockOutOfRange)?;
    let destination = escape(settings.idp.sso_url.as_deref().unwrap_or_default());
    let assertion_url = escape(&settings.acs_url);
    let entity_id = escape(&settings.sp_entity_id);
    let name_id_format = escape(&settings.name_id_format);
    Ok(format!(
        "<samlp:AuthnRequest\n  xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\"\n  xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\"\n  ID=\"{request_id}\"\n  Version=\"2.0\"\n  IssueInstant=\"{issue_instant}\"\n  Destination=\"{destination}\"\n  ProtocolBinding=\"{HTTP_POST_BINDING}\"\n  AssertionConsumerServiceURL=\"{assertion_url}\">\n    <saml:Issuer>{entity_id}</saml:Issuer>\n    <samlp:NameIDPolicy\n        Format=\"{name_id_format}\"\n        AllowCreate=\"true\" />\n\n</samlp:AuthnRequest>"
    ))
}

pub fn redirect_location(
    sso_url: &str,
    request_xml: &str,
    relay_state: &str,
) -> Result<String, SamlProviderError> {
    if !is_http_url(sso_url) {
        return Err(SamlProviderError::InvalidRedirectUrl);
    }
    let encoded = STANDARD.encode(deflate(request_xml)?);
    let separator = if sso_url.contains('?') { '&' } else { '?' };
    Ok(format!(
        "{sso_url}{separator}SAMLRequest={}&RelayState={}",
        quote_plus(&encoded),
        quote_plus(relay_state)
    ))
}

fn deflate(value: &str) -> Result<Vec<u8>, SamlProviderError> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(value.as_bytes())
        .map_err(SamlProviderError::AuthnRequest)?;
    encoder.finish().map_err(SamlProviderError::AuthnRequest)
}

// OneLogin_Saml2_Utils.redirect: a case-insensitive "http://" or "https://" prefix, nothing more
fn is_http_url(url: &str) -> bool {
    url.get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || url
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

fn quote_plus(value: &str) -> String {
    percent_encode(value.as_bytes(), FORM_ENCODING)
        .to_string()
        .replace("%20", "+")
}
