use betterlite_error::envelope::WireField;
use rstest::rstest;

use super::super::error::VectorFileWriteFailure;
use super::support::{Fixture, PROVIDER_ERROR, REQUEST_TIMEOUT, Spy, Which, call};

// "invalid_request_error" outranks the status: a 404 carrying it raises BadRequestError with null wire fields
fn assert_bad_request(failure: &VectorFileWriteFailure) {
    let raised = failure.raised().expect("BadRequestError has a projection");
    assert_eq!(raised.class, "BadRequestError");
    assert_eq!(raised.status, 404_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    // the SDK leaves code unset rather than substituting the status as a string
    assert_eq!(raised.code, None);
    // phrase mapping keeps the provider's text: the body the update's reply carried
    assert_eq!(
        raised.message,
        format!("OpenAIException - {PROVIDER_ERROR}")
    );
}

#[rstest]
#[case::update(Which::Update)]
#[case::aupdate(Which::Aupdate)]
#[case::delete(Which::Delete)]
#[case::adelete(Which::Adelete)]
#[tokio::test]
async fn file_write_error_mapping(#[case] which: Which) {
    let spy = Spy::failing(404, PROVIDER_ERROR);
    let fixture = Fixture::new();
    let failure = call(which, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider answer is an error");
    assert_bad_request(&failure);
    // the write reaches the provider once, and no retry sits behind it
    assert_eq!(spy.call_count(), 1);
}
