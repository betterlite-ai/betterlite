#![cfg(test)]

use rstest::rstest;

use super::{HeaderCarrier, validate_header_name};

#[rstest]
fn header_carrier_conveyance() {
    let default_carrier = HeaderCarrier::default();
    assert_eq!(default_carrier.header_name, "Authorization");
    assert_eq!(default_carrier.value_prefix, "Bearer");
    assert_eq!(
        default_carrier.header("secret-token"),
        ("Authorization".to_owned(), "Bearer secret-token".to_owned())
    );

    let custom_carrier = HeaderCarrier {
        header_name: validate_header_name("X-Api-Key").expect("valid token header name"),
        value_prefix: String::new(),
    };
    assert_eq!(
        custom_carrier.header("raw-value"),
        ("X-Api-Key".to_owned(), "raw-value".to_owned())
    );
}

#[rstest]
#[case::space(" ")]
#[case::colon("X:Api-Key")]
#[case::crlf("X-Api\r\nKey")]
#[case::empty("")]
fn validate_header_name_rejects_invalid_names(#[case] raw: &str) {
    validate_header_name(raw).unwrap_err();
}

#[rstest]
fn validate_header_name_trims_and_preserves_case() {
    assert_eq!(
        validate_header_name("  X-Custom-Header  ").expect("trims to a valid token"),
        "X-Custom-Header"
    );
    let error = validate_header_name("bad header").unwrap_err();
    assert_eq!(
        error.summary(),
        "misconfigured: invalid upstream header name: 'bad header'"
    );
}
