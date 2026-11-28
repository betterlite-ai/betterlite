use crate::saml_provider::{DS_NAMESPACE, MD_NAMESPACE};

use super::parse::{Element, decode_document};
use super::{IdpMetadataError, IdpSettings, X509CertMulti};

const HTTP_REDIRECT_BINDING: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
const ENTITY_DESCRIPTOR: &[u8] = b"EntityDescriptor";
const IDP_SSO_DESCRIPTOR: &[u8] = b"IDPSSODescriptor";
const NAME_ID_FORMAT: &[u8] = b"NameIDFormat";
const SINGLE_SIGN_ON_SERVICE: &[u8] = b"SingleSignOnService";
const SINGLE_LOGOUT_SERVICE: &[u8] = b"SingleLogoutService";
const KEY_DESCRIPTOR: &[u8] = b"KeyDescriptor";
const KEY_INFO: &[u8] = b"KeyInfo";
const X509_DATA: &[u8] = b"X509Data";
const X509_CERTIFICATE: &[u8] = b"X509Certificate";

pub fn decode_saml_metadata(xml: &[u8]) -> Result<Option<IdpSettings>, IdpMetadataError> {
    decode_document(xml)?
        .find_descendant(MD_NAMESPACE, ENTITY_DESCRIPTOR)
        .map_or(Ok(None), extract_idp)
}

fn extract_idp(entity: &Element) -> Result<Option<IdpSettings>, IdpMetadataError> {
    let Some(descriptor) = entity
        .children_named(MD_NAMESPACE, IDP_SSO_DESCRIPTOR)
        .next()
    else {
        return Ok(None);
    };
    let mut idp = IdpSettings {
        entity_id: entity.attribute(b"entityID").map(str::to_owned),
        sso_url: binding_location(descriptor, SINGLE_SIGN_ON_SERVICE),
        slo_url: binding_location(descriptor, SINGLE_LOGOUT_SERVICE),
        x509_cert: None,
        x509_cert_multi: None,
        name_id_format: descriptor
            .children_named(MD_NAMESPACE, NAME_ID_FORMAT)
            .next()
            .and_then(|node| node.text.clone()),
        want_authn_requests_signed: descriptor
            .attribute(b"WantAuthnRequestsSigned")
            .map(|value| value == "true"),
    };
    apply_certificates(&mut idp, descriptor)?;
    Ok(Some(idp))
}

// LiteLLM keeps a single x509cert only when one signing or one encryption certificate exists, or both are the same.
fn apply_certificates(idp: &mut IdpSettings, descriptor: &Element) -> Result<(), IdpMetadataError> {
    let mut signing = Vec::new();
    let mut encryption = Vec::new();
    for key_descriptor in descriptor.children_named(MD_NAMESPACE, KEY_DESCRIPTOR) {
        let usage = key_descriptor.attribute(b"use").unwrap_or_default();
        let mut texts = Vec::new();
        for x509_data in key_descriptor
            .children_named(DS_NAMESPACE, KEY_INFO)
            .flat_map(|key_info| key_info.children_named(DS_NAMESPACE, X509_DATA))
        {
            for certificate in x509_data.children_named(DS_NAMESPACE, X509_CERTIFICATE) {
                let text = certificate
                    .text
                    .as_deref()
                    .ok_or(IdpMetadataError::CertificateText)?;
                texts.push(text.split_whitespace().collect::<String>());
            }
        }
        if !usage.contains("encryption") {
            signing.extend(texts.iter().cloned());
        }
        if !usage.contains("signing") {
            encryption.extend(texts);
        }
    }
    match (signing.as_slice(), encryption.as_slice()) {
        ([certificate], []) | ([], [certificate]) => idp.x509_cert = Some(certificate.clone()),
        ([signing_certificate], [encryption_certificate])
            if signing_certificate == encryption_certificate =>
        {
            idp.x509_cert = Some(signing_certificate.clone());
        }
        ([_], [_]) | ([], []) | ([_, _, ..], _) | (_, [_, _, ..]) => {}
    }
    if idp.x509_cert.is_none() && (!signing.is_empty() || !encryption.is_empty()) {
        idp.x509_cert_multi = Some(X509CertMulti {
            signing: Some(signing).filter(|values| !values.is_empty()),
            encryption: Some(encryption).filter(|values| !values.is_empty()),
        });
    }
    Ok(())
}

fn binding_location(descriptor: &Element, local: &[u8]) -> Option<String> {
    let service = descriptor
        .children_named(MD_NAMESPACE, local)
        .find(|node| node.attribute(b"Binding") == Some(HTTP_REDIRECT_BINDING))?;
    service.attribute(b"Location").map(str::to_owned)
}
