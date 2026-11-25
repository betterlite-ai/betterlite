#![cfg(test)]

use std::error::Error as _;
use std::sync::Arc;

use super::StreamFailure;

#[test]
fn error_keeps_its_source() {
    let failure = StreamFailure::upstream(
        "UpstreamReadError".to_owned(),
        "read failed".to_owned(),
        "Traceback: upstream".to_owned(),
        Arc::new(std::io::Error::other("socket reset")),
    );
    let source = failure.source().and_then(|error| error.source()).unwrap();
    assert_eq!(source.to_string(), "socket reset");
    assert_eq!(failure.to_string(), "read failed");
    assert_eq!(failure.class(), "UpstreamReadError");
    assert_eq!(failure.message(), "read failed");
    assert_eq!(failure.traceback(), "Traceback: upstream");
}
