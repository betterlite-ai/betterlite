#![cfg(test)]

use super::*;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use rstest::rstest;

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("scan-key")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

#[rstest]
#[case("v2:gcm:not-decryptable", ValueClass::Migrated)]
#[case("", ValueClass::Plaintext)]
#[case("ordinary text", ValueClass::Plaintext)]
fn classification_trusts_prefix_and_plaintext(#[case] text: &str, #[case] expected: ValueClass) {
    assert_eq!(
        classify_value(&Value::String(text.to_owned()), &cipher(), "scan"),
        expected
    );
}

#[test]
fn nested_walker_is_iterative_and_only_yields_string_leaves() {
    let value = serde_json::json!({"a": ["one", {"b": "two"}], "n": 3});
    let mut leaves: Vec<&str> = iter_encrypted_strings(&value).collect();
    leaves.sort_unstable();
    assert_eq!(leaves, vec!["one", "two"]);
}

#[test]
fn migration_json_is_bounded_and_typed() {
    let value = decode_migration_json(br#"{"a":[true,null]}"#).unwrap();
    assert!(matches!(value, MigrationJson::Object(_)));
    let oversized = vec![b' '; MAX_MIGRATION_JSON_BYTES + 1];
    assert!(matches!(
        decode_migration_json(&oversized),
        Err(ScanError::InputTooLarge)
    ));
}

#[test]
fn report_wire_keys_and_checked_totals_are_stable() {
    let mut report = LocationReport::new("credentials");
    report.count(ValueClass::Legacy).unwrap();
    report.count(ValueClass::Migrated).unwrap();
    let aggregate = MigrationReport {
        locations: vec![report.clone()],
    };
    assert_eq!(aggregate.residual_legacy().unwrap(), 1);
    assert_eq!(
        report.as_dict().keys().collect::<Vec<_>>(),
        vec![
            "already_v2",
            "legacy",
            "migrated",
            "plaintext",
            "scanned",
            "undecryptable"
        ]
    );
}
