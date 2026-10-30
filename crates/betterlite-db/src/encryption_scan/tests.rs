#![cfg(test)]

use super::*;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use rstest::rstest;
use secrecy::ExposeSecret;

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
        classify_value(
            &MigrationJson::String(SecretString::from(text)),
            &cipher(),
            "scan"
        ),
        expected
    );
}

#[test]
fn secret_map_classification_matches_storage_shapes() {
    assert_eq!(
        _classify_secret_map(&serde_json::from_str("{}").unwrap(), &cipher(), "env"),
        None
    );
    assert_eq!(
        _classify_secret_map(
            &serde_json::from_str("{\"a\":\"1\"}").unwrap(),
            &cipher(),
            "env"
        ),
        Some(ValueClass::Legacy)
    );
    assert_eq!(
        _classify_secret_map(
            &MigrationJson::String(SecretString::from("{bad")),
            &cipher(),
            "env"
        ),
        Some(ValueClass::Undecryptable)
    );
    assert_eq!(
        _classify_secret_map(&MigrationJson::Object(BTreeMap::new()), &cipher(), "env"),
        None
    );
    assert_eq!(
        _classify_secret_map(
            &MigrationJson::String(SecretString::from("\"v2:gcm:map\"")),
            &cipher(),
            "env"
        ),
        Some(ValueClass::Migrated)
    );
}

#[test]
fn nested_walker_is_iterative_and_only_yields_string_leaves() {
    let value: MigrationJson = serde_json::from_str(r#"{"a":["one",{"b":"two"}],"n":3}"#).unwrap();
    let mut leaves: Vec<&str> = iter_encrypted_strings(&value)
        .map(ExposeSecret::expose_secret)
        .collect();
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
    let mut report = LocationReport {
        location: String::from("credentials"),
        scanned: 0,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: 0,
    };
    report.count(Some(ValueClass::Legacy)).unwrap();
    report.count(Some(ValueClass::Migrated)).unwrap();
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
    let projection = aggregate.as_dict().unwrap();
    assert!(projection.contains_key("residual_legacy"));
    assert!(projection.contains_key("total_undecryptable"));
    assert!(projection.contains_key("locations"));
}

#[test]
fn replays_the_migration_json_corpus() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/migration_json");
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .try_for_each(|path| decode_migration_json(&std::fs::read(path).unwrap()).map(drop))
        .unwrap();
}
