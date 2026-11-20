use betterlite_error::envelope::WireField;

use super::super::error::VectorStoreWriteFailure;
use super::support::{Fixture, PROVIDER_ERROR, REQUEST_TIMEOUT, Resolution, Spy, Which, call};

// "invalid_request_error" outranks the status: a 404 carrying it raises BadRequestError with null wire fields.
fn assert_bad_request(failure: &VectorStoreWriteFailure) {
    let raised = failure.raised().expect("BadRequestError has a projection");
    assert_eq!(raised.class, "BadRequestError");
    assert_eq!(raised.status, 404_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    // the SDK leaves code unset rather than substituting the status as a string
    assert_eq!(raised.code, None);
}

#[tokio::test]
async fn write_provider_errors() {
    let spy = Spy::failing(404, PROVIDER_ERROR);
    let resolution = Resolution::new();
    let fixture = Fixture::new();
    let update_failure = call(Which::Update, &spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider answer is an error");
    assert_bad_request(&update_failure);
    let async_failure = call(Which::Aupdate, &spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider answer is an error");
    assert_bad_request(&async_failure);
    let delete_failure = call(Which::Delete, &spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider answer is an error");
    assert_bad_request(&delete_failure);
    // one request per write, and no retry behind any of them
    assert_eq!(spy.call_count(), 3);
}
