#![cfg(test)]

use super::*;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use rstest::rstest;
use std::{fs, path::Path};

pub(super) fn cipher() -> DbCipher {
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
#[case::prefix("v2:gcm:not-decryptable", ValueClass::Migrated)]
#[case::empty("", ValueClass::Plaintext)]
#[case::plain("ordinary text", ValueClass::Plaintext)]
fn classification_trusts_prefix_and_plaintext(#[case] text: &str, #[case] expected: ValueClass) {
    assert_eq!(
        classify_value(
            &MigrationJson::String(SecretText(SecretString::from(text))),
            &cipher(),
            "scan"
        ),
        expected
    );
}

#[rstest]
#[case::empty_object("{}", None)]
#[case::empty_text_map(r#""{}""#, None)]
#[case::null("null", None)]
#[case::native_map(r#"{"a":"1"}"#, Some(ValueClass::Legacy))]
#[case::text_map(r#""{\"a\":\"1\"}""#, Some(ValueClass::Legacy))]
#[case::malformed(r#""{bad""#, Some(ValueClass::Undecryptable))]
#[case::invalid_v2(r#""v2:gcm:map""#, Some(ValueClass::Undecryptable))]
#[case::quoted_invalid_v2(r#""\"v2:gcm:map\"""#, Some(ValueClass::Undecryptable))]
#[case::wrong_shape("[]", Some(ValueClass::Undecryptable))]
fn secret_map_classification_matches_storage_shapes(
    #[case] json: &str,
    #[case] expected: Option<ValueClass>,
) {
    let value = decode_migration_json(json.as_bytes()).unwrap();
    assert_eq!(classify_secret_map(&value, &cipher(), "env"), expected);
}

#[test]
fn nested_walker_is_iterative_and_only_yields_string_leaves() {
    let value: MigrationJson = serde_json::from_str(r#"{"a":["one",{"b":"two"}],"n":3}"#).unwrap();
    let mut leaves: Vec<&str> = iter_encrypted_strings(&value)
        .map(|secret| secret.0.expose_secret())
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
        scanned: 20,
        migrated: 2,
        already_v2: 3,
        plaintext: 4,
        undecryptable: 5,
        legacy: 6,
    };
    report.count(Some(ValueClass::Legacy)).unwrap();
    report.count(Some(ValueClass::Migrated)).unwrap();
    let aggregate = MigrationReport {
        locations: vec![report.clone()],
    };
    assert_eq!(aggregate.as_dict().unwrap().residual_legacy, 7);
    assert_eq!(
        report.as_dict(),
        report::LocationCounts {
            scanned: 22,
            migrated: 2,
            already_v2: 4,
            plaintext: 4,
            undecryptable: 5,
            legacy: 7,
        }
    );
    let projection = serde_json::to_string(&aggregate.as_dict().unwrap()).unwrap();
    assert_eq!(
        projection,
        r#"{"residual_legacy":7,"total_undecryptable":5,"locations":{"credentials":{"scanned":22,"migrated":2,"already_v2":4,"plaintext":4,"undecryptable":5,"legacy":7}}}"#
    );
}

#[test]
fn replays_the_migration_json_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/migration_json");
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .for_each(|path| {
            drop(decode_migration_json(&fs::read(path).unwrap()));
        });
}

#[test]
fn callbacks_visit_both_metadata_shapes() {
    let metadata = decode_migration_json(br#"{"logging":[null,{"callback_vars":{"token":"litellm_enc::v2:gcm:bad"}}],"callback_settings":{"callback_vars":{"token":"plain"}}}"#).unwrap();
    let vars = iter_callback_var_dicts(&metadata);
    let classes: Vec<_> = vars
        .iter()
        .flat_map(|entry| entry.values())
        .map(|value| classify_callback_value(value, &cipher()))
        .collect();
    assert_eq!(classes, vec![ValueClass::Migrated, ValueClass::Plaintext]);
}

#[test]
fn reencrypts_legacy_and_preserves_unreadable_values() {
    let mut engine = cipher();
    let stored = engine
        .encrypt_value(StoredValue::<()>::Text(SecretString::from("secret")), None)
        .unwrap();
    let StoredValue::Text(legacy) = stored else {
        panic!("expected text")
    };
    let value = MigrationJson::String(SecretText(legacy));
    assert_eq!(classify_value(&value, &engine, "scan"), ValueClass::Legacy);
    engine.settings.algorithm = Algorithm::Aes256Gcm;
    let rewritten = reencrypt_value(&value, &engine, "scan").unwrap();
    assert_eq!(
        classify_value(&rewritten, &engine, "scan"),
        ValueClass::Migrated
    );
    assert_eq!(
        serde_json::to_string(&reencrypt_value(&rewritten, &engine, "scan").unwrap()).unwrap(),
        serde_json::to_string(&rewritten).unwrap()
    );
    let bad = decode_migration_json(br#""unreadable""#).unwrap();
    assert_eq!(
        serde_json::to_string(&reencrypt_value(&bad, &engine, "scan").unwrap()).unwrap(),
        "\"unreadable\""
    );
}

#[test]
fn encrypted_maps_are_authenticated_before_counting() {
    let mut engine = cipher();
    engine.settings.algorithm = Algorithm::Aes256Gcm;
    let stored = engine
        .encrypt_value(StoredValue::<()>::Text(SecretString::from("{}")), None)
        .unwrap();
    let StoredValue::Text(sealed) = stored else {
        panic!("expected text")
    };
    let value = MigrationJson::String(SecretText(sealed));
    assert_eq!(classify_secret_map(&value, &engine, "env"), None);
}

#[test]
fn report_counters_and_totals_reject_overflow() {
    let mut report = LocationReport {
        location: String::from("test"),
        scanned: u64::MAX,
        migrated: 0,
        already_v2: 0,
        plaintext: 0,
        undecryptable: 0,
        legacy: u64::MAX,
    };
    assert!(matches!(
        report.count(Some(ValueClass::Plaintext)),
        Err(ScanError::CounterOverflow)
    ));
    let aggregate = MigrationReport {
        locations: vec![report.clone(), report],
    };
    assert!(matches!(
        aggregate.as_dict(),
        Err(ScanError::CounterOverflow)
    ));
}

#[test]
fn stored_strings_redact_debug_output() {
    let value = decode_migration_json(br#"{"token":"never-print-this"}"#).unwrap();
    assert!(!format!("{value:?}").contains("never-print-this"));
}
