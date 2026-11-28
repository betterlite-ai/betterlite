use chrono::DateTime;
use quick_xml::escape::escape;
use thiserror::Error;

use crate::saml_provider::{MD_NAMESPACE, SAML_PROTOCOL_NAMESPACE};

use super::error::SamlProviderError;
use super::idp_metadata::parse::{Element, decode_document};
use super::settings::MergedSettings;

const TIME_VALID_SECONDS: i64 = 172_800;
const TIME_CACHED: i64 = 604_800;
const ENTITY_DESCRIPTOR: &[u8] = b"EntityDescriptor";
const SP_SSO_DESCRIPTOR: &[u8] = b"SPSSODescriptor";

pub fn build_sp_metadata(settings: &MergedSettings, now: i64) -> Result<String, SamlProviderError> {
    let valid_until_seconds = now
        .checked_add(TIME_VALID_SECONDS)
        .ok_or(SamlProviderError::ClockOutOfRange)?;
    let valid_until =
        super::timestamp(valid_until_seconds).ok_or(SamlProviderError::ClockOutOfRange)?;
    let authn_requests_signed = settings.authn_requests_signed.0.to_string();
    let want_assertions_signed = settings.want_assertions_signed.0.to_string();
    let entity_id = escape(&settings.sp_entity_id);
    let name_id_format = escape(&settings.name_id_format);
    let acs_url = escape(&settings.acs_url);
    Ok(format!(
        "<?xml version=\"1.0\"?>\n<md:EntityDescriptor xmlns:md=\"{MD_NAMESPACE}\"\n                     validUntil=\"{valid_until}\"\n                     cacheDuration=\"PT{TIME_CACHED}S\"\n                     entityID=\"{entity_id}\">\n    <md:SPSSODescriptor AuthnRequestsSigned=\"{authn_requests_signed}\" WantAssertionsSigned=\"{want_assertions_signed}\" protocolSupportEnumeration=\"{SAML_PROTOCOL_NAMESPACE}\">\n        <md:NameIDFormat>{name_id_format}</md:NameIDFormat>\n        <md:AssertionConsumerService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST\"\n                                     Location=\"{acs_url}\"\n                                     index=\"1\" />\n    </md:SPSSODescriptor>\n\n\n</md:EntityDescriptor>"
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Invalid SP metadata: {}", .codes.join(", "))]
pub struct SpMetadataErrors {
    pub codes: Vec<&'static str>,
}

#[must_use]
pub fn validate_sp_metadata(metadata: &str, now: i64) -> SpMetadataErrors {
    let mut codes = Vec::new();
    let Ok(document) = decode_document(metadata.as_bytes()) else {
        codes.push("unloaded_xml");
        return SpMetadataErrors { codes };
    };
    if document.local.as_slice() != ENTITY_DESCRIPTOR
        || document.namespace.as_deref() != Some(MD_NAMESPACE.as_bytes())
    {
        codes.push("noEntityDescriptor_xml");
        return SpMetadataErrors { codes };
    }
    if document.count_descendants(MD_NAMESPACE, SP_SSO_DESCRIPTOR) != 1 {
        codes.push("onlySPSSODescriptor_allowed_xml");
        return SpMetadataErrors { codes };
    }
    if expiry(&document, now).is_some_and(|limit| now > limit) {
        codes.push("expired_xml");
    }
    SpMetadataErrors { codes }
}

// OneLogin_Saml2_Utils.get_expire_time: cacheDuration from now, capped by validUntil
fn expiry(root: &Element, now: i64) -> Option<i64> {
    let valid_until = root
        .attribute(b"validUntil")
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|moment| moment.timestamp());
    let cache_expiry = root
        .attribute(b"cacheDuration")
        .and_then(|text| {
            text.strip_prefix("PT")?
                .strip_suffix('S')?
                .parse::<i64>()
                .ok()
        })
        .and_then(|seconds| now.checked_add(seconds));
    match (cache_expiry, valid_until) {
        (Some(cache), Some(valid)) => Some(cache.min(valid)),
        (cache, valid) => cache.or(valid),
    }
}
