mod tests;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;

// _verify_pkce: SHA-256 of the verifier's UTF-8 bytes, base64url without padding, compared in constant time.
#[must_use]
pub fn verify_pkce(code_verifier: &str, code_challenge: &str) -> bool {
    let digest = Sha256::digest(code_verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    bool::from(computed.as_bytes().ct_eq(code_challenge.as_bytes()))
}
