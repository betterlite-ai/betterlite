#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{
    GENERIC_CLIENT_ID_ENV, GOOGLE_CLIENT_ID_ENV, MICROSOFT_CLIENT_ID_ENV,
    SAML_IDP_METADATA_URL_ENV, SAML_IDP_METADATA_XML_ENV, load_bridge_settings,
};

type Presence = (
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(&[], (None, None, None, None, None))]
#[case::google(&[(GOOGLE_CLIENT_ID_ENV, "google-id")], (Some("google-id"), None, None, None, None))]
#[case::microsoft(&[(MICROSOFT_CLIENT_ID_ENV, "microsoft-id")], (None, Some("microsoft-id"), None, None, None))]
#[case::generic(&[(GENERIC_CLIENT_ID_ENV, "generic-id")], (None, None, Some("generic-id"), None, None))]
#[case::empty_google_is_present(&[(GOOGLE_CLIENT_ID_ENV, "")], (Some(""), None, None, None, None))]
#[case::empty_microsoft_is_present(&[(MICROSOFT_CLIENT_ID_ENV, "")], (None, Some(""), None, None, None))]
#[case::empty_generic_is_present(&[(GENERIC_CLIENT_ID_ENV, "")], (None, None, Some(""), None, None))]
#[case::saml_url(&[(SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata")], (None, None, None, Some("https://idp.example/metadata"), None))]
#[case::saml_xml(&[(SAML_IDP_METADATA_XML_ENV, "<xml/>")], (None, None, None, None, Some("<xml/>")))]
#[case::empty_saml_url_is_absent(&[(SAML_IDP_METADATA_URL_ENV, "")], (None, None, None, None, None))]
#[case::empty_saml_xml_is_absent(&[(SAML_IDP_METADATA_XML_ENV, "")], (None, None, None, None, None))]
#[case::saml_spaces_are_kept(&[(SAML_IDP_METADATA_XML_ENV, " ")], (None, None, None, None, Some(" ")))]
#[case::every_provider(
    &[(GOOGLE_CLIENT_ID_ENV, ""), (MICROSOFT_CLIENT_ID_ENV, ""), (GENERIC_CLIENT_ID_ENV, ""), (SAML_IDP_METADATA_URL_ENV, "https://idp.example/metadata")],
    (Some(""), Some(""), Some(""), Some("https://idp.example/metadata"), None)
)]
fn loads_provider_presence(#[case] pairs: &[(&str, &str)], #[case] expected: Presence) {
    let settings = load_bridge_settings(&environment(pairs));
    assert_eq!(
        (
            settings.google_client_id.as_deref(),
            settings.microsoft_client_id.as_deref(),
            settings.generic_client_id.as_deref(),
            settings.saml_idp_metadata_url.as_deref(),
            settings.saml_idp_metadata_xml.as_deref(),
        ),
        expected
    );
}
