use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;

use super::super::encryption::encrypt_credential_values;
use super::super::error::CredentialError;
use super::support::{
    cipher, cipher_with, credential, decrypt_text, keyless_cipher, number, open_text, payload, text,
};

#[test]
fn encryption_seals_strings_and_passes_other_values_through() {
    let original = credential(
        "openai",
        &[("team", text("ml"))],
        &[("api_key", text("sk-abcdefgh")), ("port", number(443))],
    );
    let encrypted = encrypt_credential_values(&cipher(), &original, None).expect("the values seal");
    let JsonData::String(sealed) = &encrypted.fields.credential_values.0["api_key"] else {
        panic!("a sealed value is text");
    };
    assert_ne!("sk-abcdefgh", sealed);
    assert_eq!("sk-abcdefgh", decrypt_text(sealed));
    assert_eq!(number(443), encrypted.fields.credential_values.0["port"]);
    assert_eq!("openai", encrypted.fields.base.credential_name);
    assert_eq!(
        payload(&[("team", text("ml"))]),
        encrypted.fields.base.credential_info
    );
}

#[test]
fn encryption_leaves_the_input_untouched() {
    let original = credential("openai", &[], &[("api_key", text("sk-abcdefgh"))]);
    let encrypted = encrypt_credential_values(&cipher(), &original, None).expect("the values seal");
    assert_eq!(
        text("sk-abcdefgh"),
        original.fields.credential_values.0["api_key"]
    );
    assert_ne!(
        original.fields.credential_values,
        encrypted.fields.credential_values
    );
}

#[test]
fn an_override_key_seals_under_that_key() {
    let original = credential("openai", &[], &[("api_key", text("sk-abcdefgh"))]);
    let other_key = SecretString::from("other-signing-key");
    let encrypted =
        encrypt_credential_values(&cipher(), &original, Some(&other_key)).expect("the values seal");
    let JsonData::String(sealed) = &encrypted.fields.credential_values.0["api_key"] else {
        panic!("a sealed value is text");
    };
    assert_eq!(
        "sk-abcdefgh",
        open_text(&cipher_with("other-signing-key"), sealed)
    );
    assert_ne!("sk-abcdefgh", decrypt_text(sealed));
}

#[test]
fn a_cipher_without_a_key_reports_encryption_failure() {
    let original = credential("openai", &[], &[("api_key", text("sk-abcdefgh"))]);
    let result = encrypt_credential_values(&keyless_cipher(), &original, None);
    assert!(matches!(result, Err(CredentialError::Encryption { .. })));
}
