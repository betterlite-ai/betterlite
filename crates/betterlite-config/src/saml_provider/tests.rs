#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sso_identity_bridge::{SAML_IDP_METADATA_URL_ENV, SAML_IDP_METADATA_XML_ENV};

use super::{
    DEFAULT_NAME_ID_FORMAT, SAML_ASSERTIONS_SIGNED_ENV, SAML_AUTHN_REQUESTS_SIGNED_ENV,
    SAML_SP_ENTITY_ID_ENV, SAML_SP_NAME_ID_FORMAT_ENV, SAML_STRICT_ENV, SAML_VALIDATE_CERT_ENV,
    SAML_WANT_MESSAGES_SIGNED_ENV, SamlFlag, SamlProviderSettings, load_saml_provider_settings,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(&[], SamlProviderSettings {
    idp_metadata_url: None,
    idp_metadata_xml: None,
    sp_entity_id: None,
    sp_name_id_format: None,
    strict: SamlFlag(true),
    want_assertions_signed: SamlFlag(true),
    want_messages_signed: SamlFlag(false),
    authn_requests_signed: SamlFlag(false),
    idp_metadata_validate_cert: SamlFlag(true),
})]
#[case::strings(
    &[
        (SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata"),
        (SAML_IDP_METADATA_XML_ENV, "<md:EntityDescriptor/>"),
        (SAML_SP_ENTITY_ID_ENV, "https://proxy.example/sso/saml/metadata"),
        (SAML_SP_NAME_ID_FORMAT_ENV, "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent"),
    ],
    SamlProviderSettings {
        idp_metadata_url: Some("https://idp.example/metadata".to_owned()),
        idp_metadata_xml: Some("<md:EntityDescriptor/>".to_owned()),
        sp_entity_id: Some("https://proxy.example/sso/saml/metadata".to_owned()),
        sp_name_id_format: Some("urn:oasis:names:tc:SAML:2.0:nameid-format:persistent".to_owned()),
        strict: SamlFlag(true),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp_metadata_validate_cert: SamlFlag(true),
    }
)]
#[case::empty_strings_are_kept(
    &[
        (SAML_IDP_METADATA_URL_ENV, ""),
        (SAML_IDP_METADATA_XML_ENV, ""),
        (SAML_SP_ENTITY_ID_ENV, ""),
        (SAML_SP_NAME_ID_FORMAT_ENV, ""),
    ],
    SamlProviderSettings {
        idp_metadata_url: Some(String::new()),
        idp_metadata_xml: Some(String::new()),
        sp_entity_id: Some(String::new()),
        sp_name_id_format: Some(String::new()),
        strict: SamlFlag(true),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp_metadata_validate_cert: SamlFlag(true),
    }
)]
#[case::booleans_parse_the_first_spelling_set(
    &[
        (SAML_STRICT_ENV, "false"),
        (SAML_ASSERTIONS_SIGNED_ENV, "0"),
        (SAML_WANT_MESSAGES_SIGNED_ENV, "yes"),
        (SAML_AUTHN_REQUESTS_SIGNED_ENV, "on"),
        (SAML_VALIDATE_CERT_ENV, " FALSE "),
    ],
    SamlProviderSettings {
        idp_metadata_url: None,
        idp_metadata_xml: None,
        sp_entity_id: None,
        sp_name_id_format: None,
        strict: SamlFlag(false),
        want_assertions_signed: SamlFlag(false),
        want_messages_signed: SamlFlag(true),
        authn_requests_signed: SamlFlag(true),
        idp_metadata_validate_cert: SamlFlag(false),
    }
)]
#[case::booleans_parse_the_second_spelling_set(
    &[
        (SAML_STRICT_ENV, "no"),
        (SAML_ASSERTIONS_SIGNED_ENV, "off"),
        (SAML_WANT_MESSAGES_SIGNED_ENV, "1"),
        (SAML_AUTHN_REQUESTS_SIGNED_ENV, "TRUE"),
        (SAML_VALIDATE_CERT_ENV, "true"),
    ],
    SamlProviderSettings {
        idp_metadata_url: None,
        idp_metadata_xml: None,
        sp_entity_id: None,
        sp_name_id_format: None,
        strict: SamlFlag(false),
        want_assertions_signed: SamlFlag(false),
        want_messages_signed: SamlFlag(true),
        authn_requests_signed: SamlFlag(true),
        idp_metadata_validate_cert: SamlFlag(true),
    }
)]
#[case::empty_booleans_take_the_default(
    &[
        (SAML_STRICT_ENV, ""),
        (SAML_ASSERTIONS_SIGNED_ENV, " "),
        (SAML_WANT_MESSAGES_SIGNED_ENV, ""),
        (SAML_AUTHN_REQUESTS_SIGNED_ENV, ""),
        (SAML_VALIDATE_CERT_ENV, ""),
    ],
    SamlProviderSettings {
        idp_metadata_url: None,
        idp_metadata_xml: None,
        sp_entity_id: None,
        sp_name_id_format: None,
        strict: SamlFlag(true),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp_metadata_validate_cert: SamlFlag(true),
    }
)]
fn loads_saml_provider_settings(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: SamlProviderSettings,
) {
    assert_eq!(
        load_saml_provider_settings(&environment(pairs)).unwrap(),
        expected
    );
}

#[rstest]
#[case::strict(SAML_STRICT_ENV)]
#[case::assertions(SAML_ASSERTIONS_SIGNED_ENV)]
#[case::messages(SAML_WANT_MESSAGES_SIGNED_ENV)]
#[case::authn_requests(SAML_AUTHN_REQUESTS_SIGNED_ENV)]
#[case::validate_cert(SAML_VALIDATE_CERT_ENV)]
fn a_bad_boolean_names_its_variable(#[case] name: &str) {
    let error = load_saml_provider_settings(&environment(&[(name, "maybe")])).unwrap_err();
    assert_eq!(
        error,
        EnvError::Bool {
            name: name.to_owned()
        }
    );
}

#[test]
fn the_default_name_id_format_is_the_email_address_urn() {
    assert_eq!(
        DEFAULT_NAME_ID_FORMAT,
        "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress"
    );
}

#[test]
fn replays_the_config_yaml_corpus() {
    let lookup: BTreeMap<String, String> = BTreeMap::from([(
        "SAML_IDP_METADATA_URL".to_owned(),
        "https://idp.example/metadata".to_owned(),
    )]);
    let valid = include_str!("../../fuzz/corpus/config_yaml/saml-provider-valid.yaml");
    ConfigLoader {
        lookup: lookup.clone(),
    }
    .load(valid)
    .unwrap();
    let invalid = include_str!("../../fuzz/corpus/config_yaml/saml-provider-invalid.yaml");
    let error = ConfigLoader { lookup }.load(invalid).unwrap_err();
    assert_eq!(error.category(), "missing_environment");
}

#[rstest]
#[case::absent(None, None, false)]
#[case::url(Some("https://idp.example/metadata"), None, true)]
#[case::xml(None, Some("<md:EntityDescriptor/>"), true)]
#[case::empty_url(Some(""), None, false)]
#[case::empty_xml(None, Some(""), false)]
#[case::empty_url_with_xml(Some(""), Some("<md:EntityDescriptor/>"), true)]
fn is_saml_configured_matches_the_toolkit(
    #[case] url: Option<&str>,
    #[case] xml: Option<&str>,
    #[case] configured: bool,
) {
    let settings = SamlProviderSettings {
        idp_metadata_url: url.map(str::to_owned),
        idp_metadata_xml: xml.map(str::to_owned),
        sp_entity_id: None,
        sp_name_id_format: None,
        strict: SamlFlag(true),
        want_assertions_signed: SamlFlag(true),
        want_messages_signed: SamlFlag(false),
        authn_requests_signed: SamlFlag(false),
        idp_metadata_validate_cert: SamlFlag(true),
    };
    assert_eq!(settings.is_configured(), configured);
}
