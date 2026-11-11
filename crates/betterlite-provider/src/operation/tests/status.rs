use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;
use rstest::rstest;

use crate::operation::{OperationError, success};

fn answer(status: u16) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: b"{}".to_vec(),
    }
}

#[rstest]
#[case::just_below(399, true)]
#[case::at_the_limit(400, false)]
#[case::server_error(503, false)]
#[case::ok(200, true)]
fn provider_framework_tests_behavior(#[case] status: u16, #[case] passes: bool) {
    let checked = success(answer(status));
    assert_eq!(checked.is_ok(), passes);
    if let Err(OperationError::Provider { response }) = checked {
        assert_eq!(response.status, status);
    }
}
