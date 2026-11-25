#![cfg(test)]

use std::error::Error as _;
use std::io;
use std::sync::Arc;

use super::StreamFailure;

#[test]
fn error_keeps_its_source() {
    let failure = StreamFailure::upstream(
        "UpstreamReadError".to_owned(),
        "read failed".to_owned(),
        "Traceback: upstream".to_owned(),
        Arc::new(io::Error::other("socket reset")),
    );
    let source = failure.source().unwrap();
    assert_eq!(source.to_string(), "socket reset");
    let upstream = source.source().unwrap();
    assert!(upstream.downcast_ref::<io::Error>().is_some());
    assert!(upstream.source().is_none());
    assert_eq!(failure.to_string(), "read failed");
    assert_eq!(failure.class(), "UpstreamReadError");
    assert_eq!(failure.message(), "read failed");
    assert_eq!(failure.traceback(), "Traceback: upstream");
}
