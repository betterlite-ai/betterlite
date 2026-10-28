pub mod app_crypto;

// Fuzz corpora and database columns hand back raw bytes with no UTF-8 guarantee.
pub(crate) fn as_utf8(bytes: &[u8]) -> Option<&str> {
    let parsed = str::from_utf8(bytes);
    parsed.ok()
}

mod tests;
