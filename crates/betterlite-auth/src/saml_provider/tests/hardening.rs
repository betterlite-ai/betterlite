use std::sync::{Arc, Mutex};

use betterlite_config::saml_provider::SamlFlag;
use betterlite_models::decisions::Json;

use super::{NOW, Recorded, TestMemory, TestTier};
use crate::auth_cache::AuthCache;
use crate::saml_provider::authn_request::authn_request_xml;
use crate::saml_provider::cache::SamlCache;
use crate::saml_provider::idp_metadata::parse::decode_document;
use crate::saml_provider::idp_metadata::{
    IdpMetadataError, IdpSettings, X509CertMulti, decode_saml_metadata,
};
use crate::saml_provider::settings::MergedSettings;
use crate::saml_provider::sp_metadata::validate_sp_metadata;

#[test]
fn a_child_in_another_namespace_is_not_the_identity_provider() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" xmlns:x="urn:example:x" entityID="urn:e"><x:IDPSSODescriptor/><md:IDPSSODescriptor><md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://md.example/sso"/></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.sso_url.as_deref(), Some("https://md.example/sso"));
}

#[test]
fn the_element_cap_admits_exactly_a_hundred_thousand_elements() {
    let children = "<md:IDPSSODescriptor/>".repeat(99_999);
    let xml = format!(
        r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata">{children}</md:EntityDescriptor>"#
    );
    assert!(
        decode_saml_metadata(xml.as_bytes())
            .unwrap()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn predefined_entities_decode_and_unknown_ones_are_refused() {
    let xml = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor><md:NameIDFormat>a&amp;b</md:NameIDFormat></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let idp = decode_saml_metadata(xml.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.name_id_format.as_deref(), Some("a&b"));
    let unknown = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"><md:IDPSSODescriptor><md:NameIDFormat>&nbsp;</md:NameIDFormat></md:IDPSSODescriptor></md:EntityDescriptor>"#;
    let error = decode_saml_metadata(unknown.as_bytes()).unwrap_err();
    assert!(matches!(error, IdpMetadataError::Entity { .. }));
}

#[test]
fn namespace_declarations_are_not_attributes() {
    let xml = r#"<md:EntityDescriptor xmlns="urn:default" xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="urn:e"/>"#;
    let document = decode_document(xml.as_bytes()).unwrap();
    assert_eq!(
        document.attributes,
        vec![(b"entityID".to_vec(), "urn:e".to_owned())]
    );
}

#[test]
fn a_declaration_before_the_root_is_read_and_one_after_it_is_refused() {
    let leading = r#"<?xml version="1.0"?><md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="urn:e"/>"#;
    let idp = decode_saml_metadata(leading.as_bytes()).unwrap().unwrap();
    assert_eq!(idp.entity_id.as_deref(), Some("urn:e"));
    let trailing = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="urn:e"/><?xml version="1.0"?>"#;
    decode_saml_metadata(trailing.as_bytes()).unwrap_err();
}

fn merged(
    idp: IdpSettings,
    want_assertions_signed: bool,
    want_messages_signed: bool,
) -> MergedSettings {
    MergedSettings {
        strict: SamlFlag(true),
        sp_entity_id: "https://proxy.example/sso/saml/metadata".to_owned(),
        acs_url: "https://proxy.example/sso/saml/callback".to_owned(),
        name_id_format: "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress".to_owned(),
        want_assertions_signed: SamlFlag(want_assertions_signed),
        want_messages_signed: SamlFlag(want_messages_signed),
        authn_requests_signed: SamlFlag(false),
        idp,
    }
}

fn idp_with(entity_id: Option<&str>) -> IdpSettings {
    IdpSettings {
        entity_id: entity_id.map(str::to_owned),
        sso_url: Some("https://idp.example.com/sso".to_owned()),
        slo_url: None,
        x509_cert: Some("CERT".to_owned()),
        x509_cert_multi: None,
        name_id_format: None,
        want_authn_requests_signed: None,
    }
}

#[test]
fn a_single_logout_url_is_checked_only_when_present_and_not_empty() {
    let invalid = merged(
        IdpSettings {
            slo_url: Some("not a url".to_owned()),
            ..idp_with(Some("urn:example:idp"))
        },
        true,
        false,
    );
    assert_eq!(
        invalid.validate(false).unwrap_err().codes,
        vec!["idp_slo_url_invalid"]
    );
    let empty = merged(
        IdpSettings {
            slo_url: Some(String::new()),
            ..idp_with(Some("urn:example:idp"))
        },
        true,
        false,
    );
    empty.validate(false).unwrap();
    let valid = merged(
        IdpSettings {
            slo_url: Some("https://idp.example.com/slo".to_owned()),
            ..idp_with(Some("urn:example:idp"))
        },
        true,
        false,
    );
    valid.validate(false).unwrap();
}

#[test]
fn a_multiple_certificate_counts_and_no_certificate_is_required_without_the_flags() {
    let multiple = IdpSettings {
        entity_id: Some("urn:example:idp".to_owned()),
        sso_url: Some("https://idp.example.com/sso".to_owned()),
        slo_url: None,
        x509_cert: None,
        x509_cert_multi: Some(X509CertMulti {
            signing: Some(vec!["AA".to_owned()]),
            encryption: None,
        }),
        name_id_format: None,
        want_authn_requests_signed: None,
    };
    merged(multiple, true, false).validate(false).unwrap();
    let bare = IdpSettings {
        entity_id: Some("urn:example:idp".to_owned()),
        sso_url: Some("https://idp.example.com/sso".to_owned()),
        slo_url: None,
        x509_cert: None,
        x509_cert_multi: None,
        name_id_format: None,
        want_authn_requests_signed: None,
    };
    merged(bare, false, false).validate(false).unwrap();
}

#[test]
fn a_document_that_expires_exactly_now_is_not_expired() {
    let metadata = r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" validUntil="2026-11-30T00:53:20Z" cacheDuration="PT604800S"><md:SPSSODescriptor/></md:EntityDescriptor>"#;
    assert!(validate_sp_metadata(metadata, NOW).codes.is_empty());
}

#[test]
fn a_metadata_root_that_is_not_an_entity_descriptor_is_refused() {
    let metadata = r#"<md:SPSSODescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata"/>"#;
    assert_eq!(
        validate_sp_metadata(metadata, NOW).codes,
        vec!["noEntityDescriptor_xml"]
    );
}

#[test]
fn authn_request_values_are_escaped() {
    let idp = IdpSettings {
        entity_id: Some("urn:example:idp".to_owned()),
        sso_url: Some("https://idp.example.com/sso?a=1&b=2".to_owned()),
        slo_url: None,
        x509_cert: Some("CERT".to_owned()),
        x509_cert_multi: None,
        name_id_format: None,
        want_authn_requests_signed: None,
    };
    let mut settings = merged(idp, true, false);
    settings.sp_entity_id = "urn:a&b".to_owned();
    let xml = authn_request_xml(&settings, "ONELOGIN_FIXED", NOW).unwrap();
    assert!(xml.contains(r#"Destination="https://idp.example.com/sso?a=1&amp;b=2""#));
    assert!(xml.contains("<saml:Issuer>urn:a&amp;b</saml:Issuer>"));
}

#[tokio::test]
async fn a_cached_idp_settings_is_decoded_from_the_tier() {
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let cached: Json = serde_json::from_str(r#"{"entity_id":"urn:x"}"#).unwrap();
    let cache = AuthCache {
        ordinary: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: Some(cached.clone()),
        },
        key_objects: TestTier {
            memory: Arc::new(TestMemory),
            recorded: Arc::clone(&recorded),
            cached: Some(cached),
        },
        management_ttl_fallback: 45.0_f64,
    };
    let decoded = cache
        .get_idp_settings("saml_idp_settings:key")
        .await
        .unwrap();
    assert_eq!(
        decoded,
        Some(IdpSettings {
            entity_id: Some("urn:x".to_owned()),
            sso_url: None,
            slo_url: None,
            x509_cert: None,
            x509_cert_multi: None,
            name_id_format: None,
            want_authn_requests_signed: None,
        })
    );
}
