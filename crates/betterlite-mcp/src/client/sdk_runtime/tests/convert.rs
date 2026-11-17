use std::time::Duration;

use rmcp::ServiceError;
use rmcp::model::{ErrorCode, ErrorData, ProtocolVersion};

use crate::client::cleanup::failure::{ErrorContext, SDK_READ_TIMEOUT_CODE};
use crate::client::sdk_runtime::convert::{protocol_failure, protocol_version, request_error};
use crate::client::sdk_transport::error::SdkError;

#[test]
fn a_read_timeout_is_the_only_failure_with_the_timeout_context() {
    let timed_out = request_error(
        "tools/call",
        ServiceError::Timeout {
            timeout: Duration::from_secs(1),
        },
    );
    let failure = protocol_failure(&timed_out).expect("a timeout is a JSON-RPC failure");
    assert_eq!(failure.code, SDK_READ_TIMEOUT_CODE);
    assert_eq!(failure.context, Some(ErrorContext::Timeout));
    assert!(failure.message.contains("tools/call"));

    // a server error that reuses the timeout code without the marker has no timeout context
    let reused_code = SdkError::JsonRpc(ErrorData::new(
        ErrorCode(SDK_READ_TIMEOUT_CODE),
        "the cache is empty",
        serde_json::to_value("not the marker").ok(),
    ));
    let reused = protocol_failure(&reused_code).expect("a server error is a JSON-RPC failure");
    assert!(
        reused.context.is_none(),
        "the code alone does not make a timeout"
    );

    let without_marker = SdkError::JsonRpc(ErrorData::new(
        ErrorCode(SDK_READ_TIMEOUT_CODE),
        "the cache is empty",
        None,
    ));
    let unmarked = protocol_failure(&without_marker).expect("a server error is a JSON-RPC failure");
    assert!(
        unmarked.context.is_none(),
        "a missing marker does not make a timeout"
    );
}

#[test]
fn a_closed_connection_is_the_sdk_connection_closed_failure() {
    let closed = request_error("tools/call", ServiceError::TransportClosed);
    let failure = protocol_failure(&closed).expect("a closed connection is a JSON-RPC failure");
    assert_eq!(failure.code, -32_000_i32);
    assert_eq!(failure.message, "Connection closed");
    assert!(failure.context.is_none());
}

#[test]
fn a_service_failure_stays_a_runtime_error() {
    let error = request_error("tools/call", ServiceError::UnexpectedResponse);
    assert!(matches!(
        error,
        SdkError::Service(ServiceError::UnexpectedResponse)
    ));
    assert!(protocol_failure(&error).is_none());
    assert!(protocol_failure(&SdkError::AlreadyOpen).is_none());
}

#[test]
fn a_known_protocol_version_keeps_its_wire_name() {
    let version = protocol_version("2025-06-18").expect("a known version");
    assert_eq!(version, ProtocolVersion::V_2025_06_18);
    let unknown = protocol_version("2099-01-01").expect("an unknown version still parses");
    assert_eq!(unknown.as_str(), "2099-01-01");
    assert_eq!(
        protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE.as_str())
            .expect("the latest version with an initialize step")
            .as_str(),
        ProtocolVersion::LATEST_WITH_INITIALIZE.as_str()
    );
}
