#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::master_key_admin_auth;
use crate::user_roles::UserRole;

const ADMIN_NAME: &str = "master_key_operator";
const MASTER_KEY_TEXT: &str = "sk-master-key-for-tests";
// lowercase SHA-256 hex of MASTER_KEY_TEXT, of the empty string, and of "sk-another-master-key"
const MASTER_KEY_DIGEST: &str = "08c01c2db90383f66f247ee03e19ac5053e408174e94be6e072db43963311fbd";
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const OTHER_KEY_DIGEST: &str = "8ffae00efda00df519b1211233139edb52252c708f82f14150312cbdceb8df49";

#[test]
fn admits_the_digest_of_the_master_key_as_the_proxy_admin() {
    let master = SecretString::from(MASTER_KEY_TEXT);
    let admin = master_key_admin_auth(MASTER_KEY_DIGEST, Some(&master), ADMIN_NAME)
        .unwrap()
        .unwrap();
    let api_key = admin
        .extra
        .api_key
        .0
        .as_ref()
        .map(|held| held.expose_secret().to_owned());
    let token = admin
        .view
        .token
        .token
        .as_ref()
        .map(|held| held.expose_secret().to_owned());
    assert_eq!(api_key, Some("litellm_proxy_master_key".to_owned()));
    assert_eq!(token, Some("litellm_proxy_master_key".to_owned()));
    assert_eq!(admin.extra.user_role, Some(UserRole::ProxyAdmin));
    assert_eq!(admin.view.token.user_id.as_deref(), Some(ADMIN_NAME));
    assert!(admin.extra.provenance.via_virtual_key);
}

#[test]
fn admits_an_empty_admin_name_as_given() {
    let master = SecretString::from(MASTER_KEY_TEXT);
    let admin = master_key_admin_auth(MASTER_KEY_DIGEST, Some(&master), "")
        .unwrap()
        .unwrap();
    assert_eq!(admin.view.token.user_id.as_deref(), Some(""));
}

#[derive(Clone, Copy, Debug)]
enum Refusal {
    NoMasterKey,
    EmptyMasterKey,
    RawMasterKeyText,
    UppercaseDigest,
    OtherKeyDigest,
    NonAsciiKeyHash,
}

fn refusal_input(scenario: Refusal) -> (Option<SecretString>, String) {
    let master = SecretString::from(MASTER_KEY_TEXT);
    match scenario {
        Refusal::NoMasterKey => (None, MASTER_KEY_DIGEST.to_owned()),
        Refusal::EmptyMasterKey => (Some(SecretString::from("")), EMPTY_DIGEST.to_owned()),
        Refusal::RawMasterKeyText => (Some(master), MASTER_KEY_TEXT.to_owned()),
        Refusal::UppercaseDigest => (Some(master), MASTER_KEY_DIGEST.to_uppercase()),
        Refusal::OtherKeyDigest => (Some(master), OTHER_KEY_DIGEST.to_owned()),
        Refusal::NonAsciiKeyHash => (Some(master), "\u{e9}".repeat(32)),
    }
}

#[rstest]
#[case::no_master_key(Refusal::NoMasterKey)]
#[case::empty_master_key(Refusal::EmptyMasterKey)]
#[case::raw_master_key_text(Refusal::RawMasterKeyText)]
#[case::uppercase_digest(Refusal::UppercaseDigest)]
#[case::other_key_digest(Refusal::OtherKeyDigest)]
#[case::non_ascii_key_hash(Refusal::NonAsciiKeyHash)]
fn refuses_anything_but_the_lowercase_digest_of_a_non_empty_master_key(#[case] scenario: Refusal) {
    let (master, key_hash) = refusal_input(scenario);
    let admin = master_key_admin_auth(&key_hash, master.as_ref(), ADMIN_NAME).unwrap();
    assert!(admin.is_none());
}
