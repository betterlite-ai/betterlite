use std::borrow::Cow;
use std::io::Read as _;
use std::str;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::read::DeflateDecoder;
use percent_encoding::percent_decode_str;
use quick_xml::Error as XmlError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use url::Url;
use url::form_urlencoded;

use super::SamlIdpError;

pub(crate) const MAX_REQUEST_BYTES: usize = 16 * 1024;
pub(crate) const MAX_XML_BYTES: usize = 256 * 1024;
const MAX_ID_BYTES: usize = 1024;
const REQUEST_PARAMETER: &str = "SAMLRequest";

#[must_use]
pub fn decode_authn_request_id(encoded: &[u8]) -> Result<String, SamlIdpError> {
    if encoded.len() > MAX_REQUEST_BYTES {
        return Err(SamlIdpError::RequestTooLarge);
    }
    let text = str::from_utf8(encoded)
        .map_err(SamlIdpError::RequestText)?
        .trim();
    let value = request_parameter_value(text)?;
    let compressed = STANDARD
        .decode(value.as_bytes())
        .map_err(SamlIdpError::RequestBase64)?;
    let xml = inflate(&compressed)?;
    root_id(&xml)
}

// The captured value is a query parameter, a form body or the redirect itself, never a location to fetch.
fn request_parameter_value(text: &str) -> Result<String, SamlIdpError> {
    if text.starts_with("http://") || text.starts_with("https://") {
        let url = Url::parse(text).map_err(SamlIdpError::RequestUrl)?;
        return request_parameter(url.query_pairs()).map(Cow::into_owned);
    }
    if text.starts_with("SAMLRequest=") || text.contains("&SAMLRequest=") {
        return request_parameter(form_urlencoded::parse(text.as_bytes())).map(Cow::into_owned);
    }
    percent_decode_str(text)
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(SamlIdpError::RequestText)
}

fn request_parameter<'value, Pairs>(mut pairs: Pairs) -> Result<Cow<'value, str>, SamlIdpError>
where
    Pairs: Iterator<Item = (Cow<'value, str>, Cow<'value, str>)>,
{
    pairs
        .find(|(name, _)| name.as_ref() == REQUEST_PARAMETER)
        .map(|(_, value)| value)
        .ok_or(SamlIdpError::MissingRequestParameter)
}

fn inflate(compressed: &[u8]) -> Result<Vec<u8>, SamlIdpError> {
    let limit = u64::try_from(MAX_XML_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut xml = Vec::new();
    DeflateDecoder::new(compressed)
        .take(limit)
        .read_to_end(&mut xml)
        .map_err(SamlIdpError::Inflate)?;
    if xml.len() > MAX_XML_BYTES {
        return Err(SamlIdpError::XmlTooLarge);
    }
    Ok(xml)
}

fn root_id(xml: &[u8]) -> Result<String, SamlIdpError> {
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event().map_err(SamlIdpError::Xml)? {
            Event::Start(tag) | Event::Empty(tag) => return authn_request_id(&tag),
            Event::DocType(_) => return Err(SamlIdpError::DocType),
            Event::End(_) | Event::Eof => return Err(SamlIdpError::NotAuthnRequest),
            Event::Decl(_)
            | Event::PI(_)
            | Event::Comment(_)
            | Event::Text(_)
            | Event::CData(_)
            | Event::GeneralRef(_) => {}
        }
    }
}

fn authn_request_id(tag: &BytesStart<'_>) -> Result<String, SamlIdpError> {
    if tag.local_name().as_ref() != b"AuthnRequest".as_slice() {
        return Err(SamlIdpError::NotAuthnRequest);
    }
    let attribute = tag
        .try_get_attribute(b"ID")
        .map_err(XmlError::from)
        .map_err(SamlIdpError::Xml)?
        .ok_or(SamlIdpError::MissingRequestId)?;
    let value = attribute
        .normalized_value(XmlVersion::Implicit1_0)
        .map_err(SamlIdpError::Xml)?;
    if value.is_empty() || value.len() > MAX_ID_BYTES {
        return Err(SamlIdpError::MissingRequestId);
    }
    Ok(value.into_owned())
}
