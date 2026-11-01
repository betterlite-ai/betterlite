#![cfg(test)]

use rstest::rstest;

use super::{HeaderCarrier, validate_header_name};

#[rstest]
fn header_carrier_conveyance() {
    let default_carrier = HeaderCarrier::default();
    assert_eq!(
        default_carrier.header("secret-token"),
        ("Authorization".to_owned(), "Bearer secret-token".to_owned())
    );

    let custom_carrier =
        HeaderCarrier::new("X-Api-Key", "").expect("X-Api-Key is a valid token header name");
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
        error.to_string(),
        "misconfigured: invalid upstream header name: 'bad header'"
    );
}

#[rstest]
#[case::space(" ")]
#[case::colon("X:Api-Key")]
#[case::crlf("X-Api\r\nKey")]
#[case::empty("")]
fn header_carrier_new_rejects_invalid_names(#[case] raw: &str) {
    HeaderCarrier::new(raw, "Bearer").unwrap_err();
}

#[rstest]
fn header_carrier_new_trims_a_padded_name_at_construction() {
    let carrier = HeaderCarrier::new("  X-Custom-Header  ", "Bearer")
        .expect("trims to a valid token before storing it");
    assert_eq!(
        carrier.header("value"),
        ("X-Custom-Header".to_owned(), "Bearer value".to_owned())
    );
}
