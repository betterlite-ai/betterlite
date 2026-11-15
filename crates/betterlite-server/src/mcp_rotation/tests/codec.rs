#![cfg(test)]

use betterlite_db::mcp_server_encoding::credentials::CredentialBlob;
use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_secrets::error::SecretError;
use rstest::rstest;

use super::super::codec::{
    decode_user_credential, decrypt_credentials, encrypt_credentials, is_falsey,
    reencrypt_credentials_blob, reencrypt_user_credential, reencrypt_user_env_var,
};
use super::super::error::RotationCodecError;
use super::support::{NEW_KEY, OLD_KEY, blob, cipher, json, new_key, object, open, sealed, string};

const SECRET_FIELDS: [&str; 7] = [
    "auth_value",
    "client_id",
    "client_secret",
    "client_private_key",
    "aws_access_key_id",
    "aws_secret_access_key",
    "aws_session_token",
];

fn encrypted_fields() -> CredentialBlob {
    let old = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    for field in SECRET_FIELDS {
        credentials.insert(
            field.to_owned(),
            JsonValue::String(sealed(&format!("{field}-value"), OLD_KEY)),
        );
    }
    credentials.insert("aws_region_name".to_owned(), string("us-east-1"));
    credentials.insert("aws_service_name".to_owned(), string("bedrock-agentcore"));
    credentials.insert("scopes".to_owned(), json(r#"["read","write"]"#));
    credentials
}

#[rstest]
#[case::auth_value("auth_value")]
#[case::client_id("client_id")]
#[case::client_secret("client_secret")]
#[case::client_private_key("client_private_key")]
#[case::aws_access_key_id("aws_access_key_id")]
#[case::aws_secret_access_key("aws_secret_access_key")]
#[case::aws_session_token("aws_session_token")]
fn credential_field_matrix(#[case] field: &str) {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let rotated =
        reencrypt_credentials_blob(Some(&JsonValue::Object(encrypted_fields())), &old, &key)
            .unwrap()
            .unwrap();
    let JsonValue::String(text) = rotated else {
        panic!("the rotated credentials are a JSON text string");
    };
    let rotated = blob(&text);
    assert_eq!(
        rotated.get(field),
        Some(&string(&sealed(&format!("{field}-value"), NEW_KEY))),
        "the {field} field is re-encrypted under the new key"
    );
}

#[test]
fn credential_field_matrix_leaves_other_fields() {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let rotated =
        reencrypt_credentials_blob(Some(&JsonValue::Object(encrypted_fields())), &old, &key)
            .unwrap()
            .unwrap();
    let JsonValue::String(text) = rotated else {
        panic!("the rotated credentials are a JSON text string");
    };
    let rotated = blob(&text);
    assert_eq!(rotated.get("aws_region_name"), Some(&string("us-east-1")));
    assert_eq!(
        rotated.get("aws_service_name"),
        Some(&string("bedrock-agentcore"))
    );
    assert_eq!(rotated.get("scopes"), Some(&json(r#"["read","write"]"#)));
}

#[test]
fn credential_field_matrix_leaves_null_and_non_string_fields() {
    let old = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    credentials.insert("auth_value".to_owned(), JsonValue::Null);
    credentials.insert("client_id".to_owned(), json("17"));
    credentials.insert("client_secret".to_owned(), json(r#"{"nested":"map"}"#));
    let rotated =
        reencrypt_credentials_blob(Some(&JsonValue::Object(credentials)), &old, &new_key())
            .unwrap()
            .unwrap();
    let JsonValue::String(text) = rotated else {
        panic!("the rotated credentials are a JSON text string");
    };
    let rotated = blob(&text);
    assert_eq!(rotated.get("auth_value"), Some(&JsonValue::Null));
    assert_eq!(rotated.get("client_id"), Some(&json("17")));
    assert_eq!(
        rotated.get("client_secret"),
        Some(&json(r#"{"nested":"map"}"#))
    );
}

#[test]
fn credential_decrypt_passthrough() {
    let old = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    credentials.insert("client_id".to_owned(), string("plain-text-not-ciphertext"));
    credentials.insert(
        "client_secret".to_owned(),
        string(&sealed("client-secret-value", OLD_KEY)),
    );
    let rotated =
        reencrypt_credentials_blob(Some(&JsonValue::Object(credentials)), &old, &new_key())
            .unwrap()
            .unwrap();
    let JsonValue::String(text) = rotated else {
        panic!("the rotated credentials are a JSON text string");
    };
    let rotated = blob(&text);
    assert_eq!(
        rotated.get("client_id"),
        Some(&string(&sealed("plain-text-not-ciphertext", NEW_KEY)))
    );
    assert_eq!(
        rotated.get("client_secret"),
        Some(&string(&sealed("client-secret-value", NEW_KEY)))
    );
}

#[rstest]
#[case::absent(None)]
#[case::null(Some(JsonValue::Null))]
#[case::empty_object(Some(object(&[])))]
#[case::empty_string(Some(string("")))]
#[case::empty_array(Some(json("[]")))]
#[case::zero(Some(json("0")))]
#[case::false_value(Some(JsonValue::Bool(false)))]
fn credential_empty_blob(#[case] empty: Option<JsonValue>) {
    let old = cipher(OLD_KEY);
    let key = new_key();
    assert_eq!(
        reencrypt_credentials_blob(empty.as_ref(), &old, &key).unwrap(),
        None
    );
}

#[rstest]
#[case::not_json(string("{not json"))]
#[case::array_text(string("[]"))]
#[case::number(json("17"))]
#[case::number_text(string("17"))]
#[case::true_value(JsonValue::Bool(true))]
fn credential_invalid_blob(#[case] invalid: JsonValue) {
    let old = cipher(OLD_KEY);
    let key = new_key();
    let result = reencrypt_credentials_blob(Some(&invalid), &old, &key);
    assert!(
        matches!(
            &result,
            Err(RotationCodecError::Encoding(_)) | Err(RotationCodecError::Json(_))
        ),
        "a non-object credential blob is rejected"
    );
}

#[test]
fn credential_json_text_blob_is_reencrypted() {
    let old = cipher(OLD_KEY);
    let inner = serde_json::to_string(&JsonValue::Object(encrypted_fields())).unwrap();
    let rotated = reencrypt_credentials_blob(Some(&string(&inner)), &old, &new_key())
        .unwrap()
        .unwrap();
    let JsonValue::String(text) = rotated else {
        panic!("the rotated credentials are a JSON text string");
    };
    let rotated = blob(&text);
    assert_eq!(
        rotated.get("auth_value"),
        Some(&string(&sealed("auth_value-value", NEW_KEY)))
    );
}

#[test]
fn encrypt_credentials_uses_the_override_key_only_for_the_secret_fields() {
    let old = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    credentials.insert("auth_value".to_owned(), string("auth"));
    credentials.insert("aws_region_name".to_owned(), string("us-east-1"));
    credentials.insert("client_id".to_owned(), JsonValue::Null);
    encrypt_credentials(&mut credentials, &old, Some(&new_key())).unwrap();
    assert_eq!(
        credentials.get("auth_value"),
        Some(&string(&sealed("auth", NEW_KEY)))
    );
    assert_eq!(
        credentials.get("aws_region_name"),
        Some(&string("us-east-1"))
    );
    assert_eq!(credentials.get("client_id"), Some(&JsonValue::Null));
}

#[test]
fn decrypt_credentials_keeps_undecryptable_strings() {
    let old = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    credentials.insert("auth_value".to_owned(), string("plain text"));
    credentials.insert(
        "client_id".to_owned(),
        string(&sealed("client-id-value", OLD_KEY)),
    );
    credentials.insert("client_secret".to_owned(), json("42"));
    decrypt_credentials(&mut credentials, &old);
    assert_eq!(credentials.get("auth_value"), Some(&string("plain text")));
    assert_eq!(
        credentials.get("client_id"),
        Some(&string("client-id-value"))
    );
    assert_eq!(credentials.get("client_secret"), Some(&json("42")));
}

#[test]
fn user_legacy_base64() {
    let old = cipher(OLD_KEY);
    let plaintext = decode_user_credential("aGVsbG8tdG9rZW4=", &old);
    assert_eq!(plaintext.as_deref(), Some("hello-token"));
    let rotated = reencrypt_user_credential("aGVsbG8tdG9rZW4=", &old, &new_key())
        .unwrap()
        .unwrap();
    let JsonValue::String(ciphertext) = rotated else {
        panic!("the rotated credential is a JSON text string");
    };
    assert_eq!(open(&ciphertext, NEW_KEY).as_deref(), Some("hello-token"));
}

#[test]
fn user_empty_plaintext() {
    let old = cipher(OLD_KEY);
    assert_eq!(decode_user_credential("", &old).as_deref(), Some(""));
    assert_eq!(decode_user_credential("***", &old).as_deref(), Some(""));
    let rotated = reencrypt_user_credential("", &old, &new_key())
        .unwrap()
        .unwrap();
    let JsonValue::String(ciphertext) = rotated else {
        panic!("the rotated credential is a JSON text string");
    };
    assert_eq!(open(&ciphertext, NEW_KEY).as_deref(), Some(""));
}

#[rstest]
#[case::short_padding("AA")]
#[case::single_char("A")]
#[case::bad_padding("AA=")]
#[case::double_group("aGVsbG8=aGVsbG8=")]
#[case::non_zero_padding("__8=")]
#[case::non_ascii("\u{e9}")]
#[case::not_base64("not a ciphertext")]
fn user_decode_error_classes(#[case] unreadable: &str) {
    let old = cipher(OLD_KEY);
    assert_eq!(
        decode_user_credential(unreadable, &old),
        None,
        "{unreadable:?} must not decode"
    );
    assert_eq!(
        reencrypt_user_credential(unreadable, &old, &new_key()).unwrap(),
        None
    );
}

#[test]
fn user_env_no_base64_fallback() {
    let old = cipher(OLD_KEY);
    assert_eq!(
        reencrypt_user_env_var("aGVsbG8tdG9rZW4=", &old, &new_key()).unwrap(),
        None
    );
    assert_eq!(
        decode_user_credential("aGVsbG8tdG9rZW4=", &old).as_deref(),
        Some("hello-token")
    );
}

#[test]
fn user_env_empty_success() {
    let old = cipher(OLD_KEY);
    let rotated = reencrypt_user_env_var("", &old, &new_key())
        .unwrap()
        .unwrap();
    let JsonValue::String(ciphertext) = rotated else {
        panic!("the rotated env vars are a JSON text string");
    };
    assert_eq!(open(&ciphertext, NEW_KEY).as_deref(), Some(""));
}

#[rstest]
#[case::null(JsonValue::Null, true)]
#[case::false_value(JsonValue::Bool(false), true)]
#[case::zero(json("0"), true)]
#[case::zero_float(json("0.0"), true)]
#[case::empty_string(string(""), true)]
#[case::empty_array(json("[]"), true)]
#[case::empty_object(object(&[]), true)]
#[case::true_value(JsonValue::Bool(true), false)]
#[case::one(json("1"), false)]
#[case::negative(json("-1"), false)]
#[case::fraction(json("0.5"), false)]
#[case::space(string(" "), false)]
#[case::array_with_null(json("[null]"), false)]
#[case::object_with_null(object(&[("a", JsonValue::Null)]), false)]
fn falsey_values_follow_python(#[case] value: JsonValue, #[case] falsey: bool) {
    assert_eq!(is_falsey(&value), falsey, "{value:?}");
}

#[test]
fn secret_error_classes_surface() {
    let cipher = cipher(OLD_KEY);
    let mut credentials = CredentialBlob::new();
    credentials.insert("auth_value".to_owned(), string("value"));
    let missing = betterlite_secrets::secret_codec::StoredCipher {
        settings: betterlite_secrets::secret_codec::CipherSettings {
            signing_key: None,
            algorithm: betterlite_secrets::secret_codec::Algorithm::XSalsa20Poly1305,
        },
        nonces: super::support::FixedNonces,
        diagnostics: betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics,
    };
    let result = encrypt_credentials(&mut credentials, &missing, None);
    assert!(matches!(result, Err(SecretError::MissingKey)));
    assert!(encrypt_credentials(&mut credentials, &cipher, Some(&new_key())).is_ok());
}
