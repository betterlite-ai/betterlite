#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_ocr::ocr_http::SecretSource as _;
use secrecy::ExposeSecret as _;

use super::EnvironmentSecrets;

#[test]
fn secret_reads_a_named_environment_value() {
    let environment = BTreeMap::from([(String::from("MISTRAL_API_KEY"), String::from("sk-test"))]);
    let source = EnvironmentSecrets { environment };
    let secret = source
        .secret("MISTRAL_API_KEY")
        .expect("the lookup answers")
        .expect("the name is set");
    assert_eq!(secret.expose_secret(), "sk-test");
}

#[test]
fn secret_reads_an_empty_value_as_unset() {
    let environment = BTreeMap::from([(String::from("MISTRAL_API_KEY"), String::new())]);
    let source = EnvironmentSecrets { environment };
    let empty = source
        .secret("MISTRAL_API_KEY")
        .expect("the lookup answers");
    assert!(empty.is_none());
    let absent = source
        .secret("AWS_ACCESS_KEY_ID")
        .expect("the lookup answers");
    assert!(absent.is_none());
}
