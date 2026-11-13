#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::langfuse_otel::destination::langfuse_authorization_header;

#[rstest]
#[case::plain("pk", "sk", "Basic cGs6c2s=")]
#[case::langfuse_style_keys("pk-lf-1", "sk-lf-2", "Basic cGstbGYtMTpzay1sZi0y")]
#[case::colon_in_the_secret("a", "b:c", "Basic YTpiOmM=")]
#[case::utf8_public_key("\u{e9}", "s", "Basic w6k6cw==")]
fn authorization_header_is_basic_base64_of_public_colon_secret(
    #[case] public_key: &str,
    #[case] secret_key: &str,
    #[case] expected: &str,
) {
    let header = langfuse_authorization_header(public_key, &SecretString::from(secret_key));
    assert_eq!(header.expose_secret(), expected);
}
