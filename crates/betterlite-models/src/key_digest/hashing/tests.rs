#![cfg(test)]

use rstest::rstest;

use super::hash_token;

const SK_ABC: &str = "1460db1b6902f8b1fc2a40d9381a24d0fd22c3bc1b2c6f999c521da73776fbe0";
const LITELLM_CLI: &str = "0c6889ac62a3282f4ea5b5f9ad6cae563198696ab8bd9c490fae581d72178bcc";

#[rstest]
#[case::plain_key("sk-abc", SK_ABC)]
#[case::cli_alias("litellm-cli", LITELLM_CLI)]
#[case::empty("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")]
#[case::utf8_bytes(
    "ключ",
    "1de36a32af798da0c1ac9297603a320ed8fe567cf21c9177112a4ce914ebb8be"
)]
fn hashes_the_utf8_bytes_to_lowercase_hex(#[case] token: &str, #[case] digest: &str) {
    assert_eq!(hash_token(token), digest);
}

#[test]
fn hashing_a_digest_again_changes_it() {
    assert_eq!(
        hash_token(SK_ABC),
        "125608c42b7910775fab830f88ae315427bfbb945e21af384f19f28d77918bfe"
    );
    assert_ne!(hash_token(SK_ABC), SK_ABC);
}
