use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

// urllib appends these four to every safe set, and quote(password, safe='') keeps nothing else
const PYTHON_QUOTE_UNSAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[must_use]
pub fn build_valkey_url(
    host: &str,
    port: u16,
    password: Option<&SecretString>,
    ssl: bool,
) -> SecretString {
    let scheme = if ssl { "rediss" } else { "redis" };
    let credentials = match password {
        Some(secret) if !secret.expose_secret().is_empty() => {
            let encoded = percent_encode(secret.expose_secret().as_bytes(), PYTHON_QUOTE_UNSAFE);
            format!(":{encoded}@")
        }
        Some(_) | None => String::new(),
    };
    SecretString::from(format!("{scheme}://{credentials}{host}:{port}"))
}

#[must_use]
pub fn pack_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len().saturating_mul(4));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

mod tests;
