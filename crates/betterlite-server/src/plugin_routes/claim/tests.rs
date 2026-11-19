use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use betterlite_pyhost::json_values::JsonObject;

use super::{
    CLAIM_TTL_SECONDS, Claim, ClaimMaterial, SALT_VARIABLE, issue_claim,
    issue_plugin_session_claim, plugin_key, verify_claim_at, verify_plugin_session_claim,
};
use crate::plugin_routes::error::PluginError;
use crate::plugin_routes::fernet::{self, FernetKey};
use crate::plugin_routes::test_support::{
    CLAIM_IV, CLAIM_TIMESTAMP, CLAIM_TOKEN, EMPTY_SALT_KEY_HEX, KEY_HEX, LONG_SALT,
    LONG_SALT_KEY_HEX, PLUGIN, SALT,
};

fn material(now: u64) -> ClaimMaterial<'static> {
    ClaimMaterial {
        salt: SALT,
        plugin: PLUGIN,
        user_id: Some("u1"),
        user_role: Some("proxy_admin"),
        now,
        iv: CLAIM_IV,
    }
}

fn python_key(hex_key: &str) -> FernetKey {
    let bytes = hex::decode(hex_key).unwrap();
    let key: [u8; 32] = bytes.try_into().unwrap();
    FernetKey::from_material(key)
}

fn sealed(plaintext: &[u8], timestamp: u64) -> String {
    let key = plugin_key(SALT, PLUGIN).unwrap();
    fernet::seal(&key, plaintext, timestamp, CLAIM_IV)
}

#[test]
fn round_trips_a_claim() {
    let token = issue_claim(&material(CLAIM_TIMESTAMP)).unwrap();
    let claim = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(claim.plugin, PLUGIN);
    assert_eq!(claim.user_id, "u1");
    assert_eq!(claim.user_role, "proxy_admin");
    assert_eq!(claim.exp, 1_700_000_030);
}

#[test]
fn verifies_the_python_claim_token() {
    let claim = verify_claim_at(SALT, PLUGIN, CLAIM_TOKEN, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(claim.plugin, PLUGIN);
    assert_eq!(claim.user_id, "u1");
    assert_eq!(claim.user_role, "proxy_admin");
    assert_eq!(claim.exp, 1_700_000_030);
}

#[test]
fn accepts_the_claim_at_its_expiry_second() {
    let claim = verify_claim_at(
        SALT,
        PLUGIN,
        CLAIM_TOKEN,
        CLAIM_TIMESTAMP + CLAIM_TTL_SECONDS,
    )
    .unwrap();
    assert_eq!(claim.user_id, "u1");
}

#[test]
fn rejects_the_claim_one_second_after_expiry() {
    let result = verify_claim_at(
        SALT,
        PLUGIN,
        CLAIM_TOKEN,
        CLAIM_TIMESTAMP + CLAIM_TTL_SECONDS + 1,
    );
    assert!(matches!(result, Err(PluginError::InvalidClaim)));
}

#[test]
fn rejects_a_claim_whose_own_exp_passed() {
    let plaintext = r#"{"plugin":"litellm-platform-plugin","user_id":"u1","user_role":"proxy_admin","exp":1699999999}"#;
    let token = sealed(plaintext.as_bytes(), CLAIM_TIMESTAMP);
    let result = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::ClaimExpired)));
}

#[test]
fn accepts_a_claim_expiring_exactly_now() {
    let plaintext = r#"{"plugin":"litellm-platform-plugin","user_id":"u1","user_role":"proxy_admin","exp":1700000000}"#;
    let token = sealed(plaintext.as_bytes(), CLAIM_TIMESTAMP);
    let claim = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(claim.exp, 1_700_000_000);
}

#[test]
fn rejects_a_claim_for_another_plugin() {
    let plaintext =
        r#"{"plugin":"other-plugin","user_id":"u1","user_role":"proxy_admin","exp":1700000030}"#;
    let token = sealed(plaintext.as_bytes(), CLAIM_TIMESTAMP);
    let result = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::AudienceMismatch)));
}

#[test]
fn rejects_a_claim_signed_under_another_salt() {
    let result = verify_claim_at("another-salt", PLUGIN, CLAIM_TOKEN, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::InvalidClaim)));
}

#[test]
fn rejects_a_tampered_claim() {
    let mut raw = URL_SAFE.decode(CLAIM_TOKEN).unwrap();
    raw[30] ^= 0x01;
    let token = URL_SAFE.encode(raw);
    let result = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::InvalidClaim)));
}

#[test]
fn rejects_a_claim_that_is_not_json() {
    let token = sealed(b"not json", CLAIM_TIMESTAMP);
    let result = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::InvalidClaim)));
}

#[test]
fn rejects_a_claim_with_another_type_for_exp() {
    let plaintext = r#"{"plugin":"litellm-platform-plugin","user_id":"u1","user_role":"proxy_admin","exp":"soon"}"#;
    let token = sealed(plaintext.as_bytes(), CLAIM_TIMESTAMP);
    let result = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP);
    assert!(matches!(result, Err(PluginError::InvalidClaim)));
}

#[test]
fn carries_only_the_four_claim_fields() {
    let key = plugin_key(SALT, PLUGIN).unwrap();
    let token = issue_claim(&material(CLAIM_TIMESTAMP)).unwrap();
    let plaintext = fernet::open(&key, &token, CLAIM_TTL_SECONDS, CLAIM_TIMESTAMP).unwrap();
    let object: JsonObject = serde_json::from_slice(&plaintext).unwrap();
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["exp", "plugin", "user_id", "user_role"]);
}

#[test]
fn leaves_the_caller_fields_empty_when_absent() {
    let mut absent = material(CLAIM_TIMESTAMP);
    absent.user_id = None;
    absent.user_role = None;
    let token = issue_claim(&absent).unwrap();
    let claim = verify_claim_at(SALT, PLUGIN, &token, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(claim.user_id, "");
    assert_eq!(claim.user_role, "");
}

#[test]
fn derives_the_python_key_for_a_plain_salt() {
    let expected = python_key(KEY_HEX);
    let token = sealed(b"payload", CLAIM_TIMESTAMP);
    let plaintext = fernet::open(&expected, &token, CLAIM_TTL_SECONDS, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(plaintext, b"payload".to_vec());
}

#[test]
fn derives_the_python_key_for_a_salt_longer_than_a_hmac_block() {
    let expected = python_key(LONG_SALT_KEY_HEX);
    let key = plugin_key(LONG_SALT, PLUGIN).unwrap();
    let token = fernet::seal(&key, b"payload", CLAIM_TIMESTAMP, CLAIM_IV);
    let plaintext = fernet::open(&expected, &token, CLAIM_TTL_SECONDS, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(plaintext, b"payload".to_vec());
}

#[test]
fn derives_the_python_key_for_an_empty_salt() {
    let expected = python_key(EMPTY_SALT_KEY_HEX);
    let key = plugin_key("", PLUGIN).unwrap();
    let token = fernet::seal(&key, b"payload", CLAIM_TIMESTAMP, CLAIM_IV);
    let plaintext = fernet::open(&expected, &token, CLAIM_TTL_SECONDS, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(plaintext, b"payload".to_vec());
}

#[test]
fn saturates_the_claim_expiry_at_the_end_of_time() {
    let token = issue_claim(&material(u64::MAX)).unwrap();
    let key = plugin_key(SALT, PLUGIN).unwrap();
    let plaintext = fernet::open(&key, &token, CLAIM_TTL_SECONDS, u64::MAX).unwrap();
    let claim: Claim = serde_json::from_slice(&plaintext).unwrap();
    assert_eq!(claim.exp, i64::MAX);
}

#[tokio::test]
async fn issues_and_verifies_through_the_environment() {
    temp_env::async_with_vars([(SALT_VARIABLE, Some(SALT))], async {
        let token = issue_plugin_session_claim(PLUGIN, Some("u1"), Some("proxy_admin")).unwrap();
        let claim = verify_plugin_session_claim(PLUGIN, &token).unwrap();
        assert_eq!(claim.user_id, "u1");
        assert_eq!(claim.user_role, "proxy_admin");
        assert_eq!(claim.plugin, PLUGIN);
    })
    .await;
}
