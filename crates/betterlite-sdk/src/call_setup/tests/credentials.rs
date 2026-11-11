use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::support::Credentials;
use super::support::request;
use crate::call_setup::credentials::Credential;
use crate::call_setup::credentials::load_credentials_from_list;

fn credential(name: &str, values: &[(&str, &str)]) -> Credential {
    let mut credential_values = IndexMap::new();
    for (key, value) in values {
        credential_values.insert((*key).to_owned(), SecretString::from((*value).to_owned()));
    }
    Credential {
        credential_name: name.to_owned(),
        credential_values,
    }
}

#[test]
fn credential_list_fills_only_missing_fields() {
    let credentials = Credentials {
        entries: vec![credential(
            "prod",
            &[("api_key", "secret"), ("api_base", "url")],
        )],
    };
    let mut request = request(r#"{"litellm_credential_name":"prod","api_key":"caller"}"#);

    let warning =
        load_credentials_from_list(&mut request, &credentials).expect("the request reads");

    assert!(warning.is_none());
    assert_eq!(
        request.get("api_key"),
        Some(&Json::Text(String::from("caller")))
    );
    assert_eq!(
        request.get("api_base"),
        Some(&Json::Text(String::from("url")))
    );
}

#[test]
fn credential_list_reports_an_unknown_name() {
    let credentials = Credentials {
        entries: Vec::new(),
    };
    let mut request = request(r#"{"litellm_credential_name":"missing"}"#);

    let warning =
        load_credentials_from_list(&mut request, &credentials).expect("the request reads");

    let reported = warning.expect("an unknown credential warns");
    assert_eq!(reported.credential_name, "missing");
}

#[test]
fn credential_list_without_a_name_does_nothing() {
    let credentials = Credentials {
        entries: vec![credential("prod", &[("api_key", "secret")])],
    };
    let mut request = request(r#"{"model":"gpt-4o"}"#);

    let warning =
        load_credentials_from_list(&mut request, &credentials).expect("the request reads");

    assert!(warning.is_none());
    assert!(request.get("api_key").is_none());
}

#[test]
fn credential_list_rejects_a_non_string_name() {
    let credentials = Credentials {
        entries: Vec::new(),
    };
    let mut request = request(r#"{"litellm_credential_name":7}"#);

    let error =
        load_credentials_from_list(&mut request, &credentials).expect_err("a number is not a name");

    assert_eq!(
        error.to_string(),
        "the litellm_credential_name value must be a string"
    );
}

#[test]
fn credential_debug_text_hides_the_values() {
    let credential = credential("prod", &[("api_key", "super-secret-value")]);

    let debug = format!("{credential:?}");

    assert!(debug.contains("api_key"));
    assert!(debug.contains("prod"));
    assert!(!debug.contains("super-secret-value"));
}
