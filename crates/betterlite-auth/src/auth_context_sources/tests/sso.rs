use betterlite_config::sections::auth_context_sources::{
    GENERIC_AUTHORIZATION_ENDPOINT_VARIABLE, GENERIC_CLIENT_ID_VARIABLE,
    GENERIC_CLIENT_SECRET_VARIABLE, GENERIC_TOKEN_ENDPOINT_VARIABLE,
    GENERIC_USERINFO_ENDPOINT_VARIABLE, GOOGLE_CLIENT_ID_VARIABLE, GOOGLE_CLIENT_SECRET_VARIABLE,
    MICROSOFT_CLIENT_ID_VARIABLE, MICROSOFT_CLIENT_SECRET_VARIABLE, MICROSOFT_TENANT_VARIABLE,
    SAML_IDP_METADATA_URL_VARIABLE, SAML_IDP_METADATA_XML_VARIABLE,
};
use rstest::rstest;
use secrecy::SecretString;

use crate::auth_context_sources::budgets::model_names_for_budget_checks;
use crate::auth_context_sources::sso::{
    SsoConfiguration, SsoRuntime, abbreviate_api_key, has_user_setup_sso,
    is_sso_provider_fully_configured,
};
use crate::auth_request_models::RequestedModels;

use super::support::env_settings;

const SAML_READY: SsoRuntime = SsoRuntime {
    saml_available: true,
};
const SAML_UNAVAILABLE: SsoRuntime = SsoRuntime {
    saml_available: false,
};

const MICROSOFT: [(&str, &str); 3] = [
    (MICROSOFT_CLIENT_ID_VARIABLE, "microsoft-client"),
    (MICROSOFT_CLIENT_SECRET_VARIABLE, "microsoft-secret"),
    (MICROSOFT_TENANT_VARIABLE, "contoso"),
];

const GENERIC: [(&str, &str); 5] = [
    (GENERIC_CLIENT_ID_VARIABLE, "generic-client"),
    (GENERIC_CLIENT_SECRET_VARIABLE, "generic-secret"),
    (
        GENERIC_AUTHORIZATION_ENDPOINT_VARIABLE,
        "https://idp.example/authorize",
    ),
    (GENERIC_TOKEN_ENDPOINT_VARIABLE, "https://idp.example/token"),
    (
        GENERIC_USERINFO_ENDPOINT_VARIABLE,
        "https://idp.example/userinfo",
    ),
];

fn configured(environment: &[(&str, &str)]) -> SsoConfiguration {
    SsoConfiguration::from(&env_settings(environment))
}

#[test]
fn sso_complete() {
    let mut environment = MICROSOFT.to_vec();
    environment.extend_from_slice(&GENERIC);
    environment.extend([
        (GOOGLE_CLIENT_ID_VARIABLE, "google-client"),
        (GOOGLE_CLIENT_SECRET_VARIABLE, "google-secret"),
        (
            SAML_IDP_METADATA_URL_VARIABLE,
            "https://idp.example/metadata",
        ),
    ]);
    let configuration = configured(&environment);
    assert!(has_user_setup_sso(&configuration));
    // the SAML runtime is absent, so the three OAuth providers alone must read as ready
    assert!(is_sso_provider_fully_configured(
        &configuration,
        SAML_UNAVAILABLE
    ));
}

#[test]
fn sso_missing_field() {
    let missing_tenant = configured(&[
        (MICROSOFT_CLIENT_ID_VARIABLE, "microsoft-client"),
        (MICROSOFT_CLIENT_SECRET_VARIABLE, "microsoft-secret"),
    ]);
    assert!(has_user_setup_sso(&missing_tenant));
    assert!(!is_sso_provider_fully_configured(
        &missing_tenant,
        SAML_READY
    ));

    let missing_secret = configured(&[
        (MICROSOFT_CLIENT_ID_VARIABLE, "microsoft-client"),
        (MICROSOFT_TENANT_VARIABLE, "contoso"),
    ]);
    assert!(has_user_setup_sso(&missing_secret));
    assert!(!is_sso_provider_fully_configured(
        &missing_secret,
        SAML_READY
    ));

    let empty_tenant = configured(&[
        (MICROSOFT_CLIENT_ID_VARIABLE, "microsoft-client"),
        (MICROSOFT_CLIENT_SECRET_VARIABLE, "microsoft-secret"),
        (MICROSOFT_TENANT_VARIABLE, ""),
    ]);
    assert!(has_user_setup_sso(&empty_tenant));
    assert!(!is_sso_provider_fully_configured(&empty_tenant, SAML_READY));
}

#[test]
fn discovery_empty_client_id() {
    let empty_client_id = configured(&[(GOOGLE_CLIENT_ID_VARIABLE, "")]);
    assert!(has_user_setup_sso(&empty_client_id));
    assert!(!is_sso_provider_fully_configured(
        &empty_client_id,
        SAML_UNAVAILABLE
    ));

    let absent = configured(&[]);
    assert!(!has_user_setup_sso(&absent));
    assert!(!is_sso_provider_fully_configured(&absent, SAML_READY));

    // an empty metadata value is false for discovery, unlike an empty client id
    let empty_metadata = configured(&[
        (SAML_IDP_METADATA_URL_VARIABLE, ""),
        (SAML_IDP_METADATA_XML_VARIABLE, ""),
    ]);
    assert!(!has_user_setup_sso(&empty_metadata));
    assert!(!is_sso_provider_fully_configured(
        &empty_metadata,
        SAML_READY
    ));
}

#[test]
fn readiness_all_companions() {
    let microsoft = configured(&MICROSOFT);
    assert!(is_sso_provider_fully_configured(
        &microsoft,
        SAML_UNAVAILABLE
    ));
    for missing in MICROSOFT {
        let reduced: Vec<(&str, &str)> = MICROSOFT
            .into_iter()
            .filter(|entry| *entry != missing)
            .collect();
        let configuration = configured(&reduced);
        assert!(
            !is_sso_provider_fully_configured(&configuration, SAML_READY),
            "microsoft without {missing:?}"
        );
    }

    let generic = configured(&GENERIC);
    assert!(is_sso_provider_fully_configured(&generic, SAML_UNAVAILABLE));
    for missing in GENERIC {
        let reduced: Vec<(&str, &str)> = GENERIC
            .into_iter()
            .filter(|entry| *entry != missing)
            .collect();
        let configuration = configured(&reduced);
        assert!(
            !is_sso_provider_fully_configured(&configuration, SAML_READY),
            "generic without {missing:?}"
        );
    }
}

#[test]
fn incomplete_provider_does_not_mask_other() {
    let configuration = configured(&[
        (MICROSOFT_CLIENT_ID_VARIABLE, "microsoft-client"),
        (GOOGLE_CLIENT_ID_VARIABLE, "google-client"),
        (GOOGLE_CLIENT_SECRET_VARIABLE, "google-secret"),
    ]);
    assert!(has_user_setup_sso(&configuration));
    assert!(is_sso_provider_fully_configured(
        &configuration,
        SAML_UNAVAILABLE
    ));
}

#[rstest]
#[case::url(&[(SAML_IDP_METADATA_URL_VARIABLE, "https://idp.example/metadata")])]
#[case::xml(&[(SAML_IDP_METADATA_XML_VARIABLE, "<EntityDescriptor/>")])]
fn saml_metadata_without_runtime(#[case] environment: &[(&str, &str)]) {
    let configuration = configured(environment);
    assert!(has_user_setup_sso(&configuration));
    assert!(!is_sso_provider_fully_configured(
        &configuration,
        SAML_UNAVAILABLE
    ));
    assert!(is_sso_provider_fully_configured(&configuration, SAML_READY));
}

#[test]
fn abbreviation_threshold_tail() {
    let key = SecretString::from("0123456789abcdef".to_owned());
    assert_eq!(abbreviate_api_key(&key, 16), "sk-...cdef");
    assert_eq!(abbreviate_api_key(&key, 17), "sk-...");

    let short = SecretString::from("0123456789abcde".to_owned());
    assert_eq!(abbreviate_api_key(&short, 16), "sk-...");
    assert_eq!(abbreviate_api_key(&short, 15), "sk-...bcde");

    let tiny = SecretString::from("abc".to_owned());
    assert_eq!(abbreviate_api_key(&tiny, 4), "sk-...");
    assert_eq!(abbreviate_api_key(&tiny, 3), "sk-...abc");

    // the threshold counts characters, not bytes
    let unicode = SecretString::from("ééééé".to_owned());
    assert_eq!(abbreviate_api_key(&unicode, 5), "sk-...éééé");
    assert_eq!(abbreviate_api_key(&unicode, 6), "sk-...");

    // a negative configured minimum is not clamped, and every key is abbreviated
    assert_eq!(abbreviate_api_key(&key, -1), "sk-...cdef");
    let empty = SecretString::from(String::new());
    assert_eq!(abbreviate_api_key(&empty, 0), "sk-...");
}

#[test]
fn model_budget_names() {
    assert_eq!(
        model_names_for_budget_checks(&RequestedModels::None),
        Vec::<String>::new()
    );
    assert_eq!(
        model_names_for_budget_checks(&RequestedModels::One("gpt-4".to_owned())),
        ["gpt-4"]
    );
    assert_eq!(
        model_names_for_budget_checks(&RequestedModels::Many(vec![
            "gpt-4".to_owned(),
            "gpt-4o".to_owned(),
        ])),
        ["gpt-4", "gpt-4o"]
    );
}

#[test]
fn budget_list_preserved() {
    let names = vec!["second".to_owned(), "first".to_owned(), "second".to_owned()];
    assert_eq!(
        model_names_for_budget_checks(&RequestedModels::Many(names.clone())),
        names
    );
}
