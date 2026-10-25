#![cfg(test)]

use rstest::rstest;
use serde_json::{Value, json};

use super::{SecretCodecInput, run_codec};

const HELLO: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj";

fn run(nonce_hex: &str, steps: &Value) -> Value {
    let input: SecretCodecInput = serde_json::from_value(json!({
        "salt_key": "salt-one",
        "master_key": null,
        "general_settings": {},
        "nonce_hex": nonce_hex,
        "steps": steps,
    }))
    .unwrap();
    run_codec(&input).expect("secret codec helper input should load")
}

#[test]
fn legacy_steps_seal_with_the_given_nonce_and_open_to_plaintext() {
    let results = run(
        "000102030405060708090a0b0c0d0e0f1011121314151617",
        &json!([
            {"op": "encrypt_value", "value": "hello", "new_key": null},
            {"op": "decrypt_value", "value": HELLO, "field": "api_key", "report": "error", "keep_original": false},
        ]),
    );
    assert_eq!(
        results,
        json!([{"value": HELLO, "logs": []}, {"value": "hello", "logs": []}])
    );
}

#[test]
fn a_short_nonce_fails_the_encryption() {
    let results = run(
        "0001",
        &json!([{"op": "encrypt_value", "value": "hello", "new_key": null}]),
    );
    assert_eq!(results[0]["error"]["class"], "ValueError");
}

#[test]
fn recorded_logs_name_the_field_only_when_one_is_given() {
    let results = run(
        "000102030405060708090a0b0c0d0e0f1011121314151617",
        &json!([
            {"op": "encrypt_value", "value": 7_i32, "new_key": null},
            {"op": "decrypt_value", "value": "AAAA-", "field": "api_key", "report": "debug", "keep_original": false},
        ]),
    );
    assert_eq!(
        results,
        json!([
            {"value": 7_i32, "logs": [{"level": "debug", "names_field": false, "salt_advice": false}]},
            {"value": null, "logs": [{"level": "debug", "names_field": true, "salt_advice": true}]},
        ])
    );
}

#[test]
fn a_plaintext_map_decodes_to_its_entries() {
    let results = run(
        "000102030405060708090a0b0c0d0e0f1011121314151617",
        &json!([{"op": "decode_map", "value": {"token": "abc"}, "field": "headers"}]),
    );
    assert_eq!(results, json!([{"value": {"token": "abc"}, "logs": []}]));
}

#[rstest]
#[case::leading_plus("+f")]
#[case::odd_length("f")]
#[case::invalid_digit("gg")]
#[case::unicode("\u{e9}")]
fn invalid_nonce_hex_is_rejected(#[case] nonce_hex: &str) {
    let input: SecretCodecInput = serde_json::from_value(json!({
        "salt_key": "salt-one",
        "master_key": null,
        "general_settings": {},
        "nonce_hex": nonce_hex,
        "steps": [],
    }))
    .unwrap();
    let error = run_codec(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
}
