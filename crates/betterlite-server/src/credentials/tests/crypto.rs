#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;

use super::super::encryption::{decrypt_credential_values, encrypt_credential_values};
use super::super::error::CredentialError;
use super::support::{
    cipher, cipher_with, credential, credential_values, decrypt_text, fields, keyless_cipher,
    number, open_text, payload, plain, row, text,
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
        payload(&[("api_key", text("sk-abcdefgh"))]),
        plain(&original.credential_values)
    );
    assert_ne!(
        plain(&original.credential_values),
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
fn encryption_seals_every_string_inside_nested_objects_and_arrays() {
    let original = credential(
        "vertex",
        &[],
        &[(
            "vertex_credentials",
            JsonData::Object(fields(&[
                ("private_key", text("secret-key")),
                ("port", number(443)),
                ("scopes", JsonData::Array(vec![text("scope-one")])),
            ])),
        )],
    );
    let encrypted = encrypt_credential_values(&cipher(), &original, None).expect("the values seal");
    let JsonData::Object(inner) = &encrypted.fields.credential_values.0["vertex_credentials"]
    else {
        panic!("the object stays an object");
    };
    let JsonData::String(sealed_key) = &inner["private_key"] else {
        panic!("a nested string seals as text");
    };
    assert_ne!("secret-key", sealed_key);
    assert_eq!("secret-key", decrypt_text(sealed_key));
    assert_eq!(number(443), inner["port"]);
    let JsonData::Array(scopes) = &inner["scopes"] else {
        panic!("the array stays an array");
    };
    let JsonData::String(sealed_scope) = &scopes[0] else {
        panic!("an array string seals as text");
    };
    assert_eq!("scope-one", decrypt_text(sealed_scope));
}

#[test]
fn decryption_restores_the_nested_plaintext() {
    let original = credential(
        "vertex",
        &[("team", text("ml"))],
        &[
            ("api_key", text("sk-abcdefgh")),
            (
                "vertex_credentials",
                JsonData::Object(fields(&[
                    ("private_key", text("secret-key")),
                    ("scopes", JsonData::Array(vec![text("scope-one")])),
                ])),
            ),
        ],
    );
    let encrypted = encrypt_credential_values(&cipher(), &original, None).expect("the values seal");
    assert_eq!(original, decrypt_credential_values(&cipher(), &encrypted));
}

#[test]
fn decryption_keeps_a_value_it_cannot_open() {
    let stored = row(&credential(
        "openai",
        &[],
        &[("api_key", text("not-ciphertext"))],
    ));
    let opened = decrypt_credential_values(&cipher(), &stored);
    assert_eq!(
        credential_values(&[("api_key", text("not-ciphertext"))]),
        opened.credential_values
    );
}

#[test]
fn a_cipher_without_a_key_reports_encryption_failure() {
    let original = credential("openai", &[], &[("api_key", text("sk-abcdefgh"))]);
    let result = encrypt_credential_values(&keyless_cipher(), &original, None);
    assert!(matches!(result, Err(CredentialError::Encryption { .. })));
}
