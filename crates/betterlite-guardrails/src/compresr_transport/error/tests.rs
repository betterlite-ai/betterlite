#![cfg(test)]

use rstest::rstest;

use super::CompresrTransportError;

#[rstest]
#[case::blocked(CompresrTransportError::BlockedHost { host: "metadata.goog".to_owned() }, None)]
#[case::scheme(CompresrTransportError::Scheme, None)]
#[case::invalid(CompresrTransportError::InvalidApiBase, None)]
#[case::no_host(CompresrTransportError::NoHost, None)]
#[case::failed(
    CompresrTransportError::CompressionFailed {
        message: "Compresr compression service returned an error"
    },
    Some(502)
)]
fn only_a_compression_failure_carries_a_status(
    #[case] error: CompresrTransportError,
    #[case] expected: Option<u16>,
) {
    assert_eq!(error.status_code(), expected);
}
