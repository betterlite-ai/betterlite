use betterlite_error::envelope::WireField;
use rstest::rstest;

use super::support::{Fixture, PROVIDER_ERROR, REQUEST_TIMEOUT, Spy, Which, call};

// the openai body mapping and the async wrappers' status-only mapping prefix their messages differently
const OPENAI_MESSAGE: &str = "OpenAIException - ";
const STATUS_MESSAGE: &str = "Exception - ";

// "invalid_request_error" outranks the status in the sync mapping; aupdate/adelete hand the outer wrap the caller's provider, none here, so the status alone decides
#[rstest]
#[case::update(Which::Update, None, "BadRequestError", OPENAI_MESSAGE)]
#[case::aupdate(Which::Aupdate, None, "NotFoundError", STATUS_MESSAGE)]
#[case::delete(Which::Delete, None, "BadRequestError", OPENAI_MESSAGE)]
#[case::adelete(Which::Adelete, None, "NotFoundError", STATUS_MESSAGE)]
#[case::aupdate_named(Which::Aupdate, Some("openai"), "BadRequestError", OPENAI_MESSAGE)]
#[case::adelete_named(Which::Adelete, Some("openai"), "BadRequestError", OPENAI_MESSAGE)]
#[tokio::test]
async fn file_write_error_mapping(
    #[case] which: Which,
    #[case] provider: Option<&str>,
    #[case] class: &str,
    #[case] message_prefix: &str,
) {
    let spy = Spy::failing(404, PROVIDER_ERROR);
    let mut fixture = Fixture::new();
    fixture.arguments.provider = provider.map(str::to_owned);
    let failure = call(which, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect_err("the provider answer is an error");
    let raised = failure.raised().expect("the failure has a projection");
    assert_eq!(raised.class, class);
    assert_eq!(raised.status, 404_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    // the SDK leaves code unset rather than substituting the status as a string
    assert_eq!(raised.code, None);
    assert!(raised.message.starts_with(message_prefix));
    // the write reaches the provider once, and no retry sits behind it
    assert_eq!(spy.call_count(), 1);
}
