#![cfg(test)]

use serde_json::{Value, json};

use super::{Input, Op};
use crate::helpers::Helper as _;

fn run(value: Value) -> Value {
    let input: Input = serde_json::from_value(value).unwrap();
    Op::run(&input).expect("stored cipher helper input should run")
}

#[test]
fn codec_steps_round_trip_through_the_fixed_nonce() {
    let results = run(json!({
        "op": "codec",
        "salt_key": "salt-one",
        "master_key": null,
        "general_settings": {},
        "nonce_hex": "000102030405060708090a0b0c0d0e0f1011121314151617",
        "steps": [
            {"kind": "encrypt_value", "value": "hello", "new_key": null},
        ],
    }));
    assert_eq!(
        results[0]["value"],
        "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXccVZjgu1TZy2W9RyeYfnLiSlXcAj"
    );
}

#[test]
fn app_crypto_roundtrips_a_json_payload() {
    let results = run(json!({
        "op": "app_crypto",
        "key": "0123456789abcdef0123456789abcde0",
        "steps": [
            {"kind": "roundtrip", "data": {"token": "sk-live"}, "aad": "caller"},
        ],
    }));
    assert_eq!(results[0]["value"], json!({"token": "sk-live"}));
}

#[test]
fn app_crypto_aad_mismatch_fails_to_decrypt() {
    let results = run(json!({
        "op": "app_crypto",
        "key": "0123456789abcdef0123456789abcde0",
        "steps": [
            {"kind": "aad_mismatch", "data": {"token": "sk-live"}, "encrypt_aad": "a", "decrypt_aad": "b"},
        ],
    }));
    assert_eq!(results[0]["error"]["class"], "ValueError");
}

#[test]
fn callback_vars_encrypt_then_decrypt_restores_the_plaintext() {
    let metadata = json!({
        "callback_settings": {"callback_vars": {"api_key": "sk-secret"}},
    });
    let encrypted = run(json!({
        "op": "callback_vars",
        "salt_key": "salt-one",
        "master_key": null,
        "general_settings": {},
        "nonce_hex": "000102030405060708090a0b0c0d0e0f1011121314151617",
        "extra": [],
        "steps": [{"kind": "encrypt", "metadata": metadata}],
    }));
    let sealed = encrypted[0]["callback_settings"]["callback_vars"]["api_key"]
        .as_str()
        .unwrap();
    assert!(sealed.starts_with("litellm_enc::"));
    let decrypted = run(json!({
        "op": "callback_vars",
        "salt_key": "salt-one",
        "master_key": null,
        "general_settings": {},
        "nonce_hex": "000102030405060708090a0b0c0d0e0f1011121314151617",
        "extra": [],
        "steps": [{"kind": "decrypt", "metadata": encrypted[0].clone()}],
    }));
    assert_eq!(
        decrypted[0]["callback_settings"]["callback_vars"]["api_key"],
        "sk-secret"
    );
}

#[test]
fn master_key_match_requires_an_exact_raw_string() {
    assert_eq!(
        run(json!({"op": "master_key_match", "presented": "sk-1", "master_key": "sk-1"})),
        Value::Bool(true)
    );
    assert_eq!(
        run(json!({"op": "master_key_match", "presented": "sk-1", "master_key": "sk-2"})),
        Value::Bool(false)
    );
    assert_eq!(
        run(json!({"op": "master_key_match", "presented": null, "master_key": "sk-1"})),
        Value::Bool(false)
    );
}
