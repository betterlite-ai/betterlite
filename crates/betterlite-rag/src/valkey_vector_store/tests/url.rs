use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::valkey_vector_store::{build_valkey_url, pack_vector};

#[rstest]
#[case::no_password(
    "valkey.example.com",
    6379,
    None,
    false,
    "redis://valkey.example.com:6379"
)]
#[case::password(
    "valkey.example.com",
    6380,
    Some("secret"),
    false,
    "redis://:secret@valkey.example.com:6380"
)]
#[case::tls_scheme(
    "valkey.example.com",
    6379,
    Some("secret"),
    true,
    "rediss://:secret@valkey.example.com:6379"
)]
#[case::empty_password_counts_as_absent(
    "valkey.example.com",
    6379,
    Some(""),
    true,
    "rediss://valkey.example.com:6379"
)]
fn url_matches_litellm(
    #[case] host: &str,
    #[case] port: u16,
    #[case] password: Option<&str>,
    #[case] ssl: bool,
    #[case] expected: &str,
) {
    let secret = password.map(SecretString::from);
    assert_eq!(
        build_valkey_url(host, port, secret.as_ref(), ssl).expose_secret(),
        expected
    );
}

#[rstest]
#[case::reserved("p@ss:w/rd", "p%40ss%3Aw%2Frd")]
#[case::unreserved_kept("a_b.c-d~e", "a_b.c-d~e")]
#[case::alphanumerics_kept("Az09", "Az09")]
#[case::space_and_percent("a b%c", "a%20b%25c")]
#[case::multibyte("\u{e9}\u{fc}", "%C3%A9%C3%BC")]
#[case::query_characters("a?b#c&d=e", "a%3Fb%23c%26d%3De")]
fn url_percent_encodes_the_password(#[case] password: &str, #[case] encoded: &str) {
    let secret = SecretString::from(password);
    assert_eq!(
        build_valkey_url("h", 1, Some(&secret), false).expose_secret(),
        format!("redis://:{encoded}@h:1")
    );
}

#[rstest]
#[case::unit_and_negative(&[1.0_f32, -2.5], &[0, 0, 128, 63, 0, 0, 32, 192])]
#[case::empty(&[], &[])]
#[case::point_one(&[0.1_f32], &[205, 204, 204, 61])]
fn pack_writes_little_endian_f32(#[case] vector: &[f32], #[case] expected: &[u8]) {
    assert_eq!(pack_vector(vector), expected);
}
