use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::privacy::{
    REDACTED_MESSAGES, REDACTED_RESPONSE, SECRET_MESSAGES, SECRET_RESPONSE, secret_payload,
};
use super::support::{
    CallLog, FakeLogger, Fixture, correlation, json_object, parse, request, run_once, state,
};
use crate::logging_success::payload_state::standard_logging_object;
use crate::logging_success::types::SuccessResult;

fn logger(name: &str, opted_out: bool, calls: &Arc<CallLog>) -> Arc<FakeLogger> {
    let mut logger = FakeLogger::new(name, calls);
    logger.message_logging = !opted_out;
    Arc::new(logger)
}

// the deprecated CustomLogger message_logging=False flag: the async hook loop redacts the shared payload
#[rstest]
#[case::async_later_logger_redacts_the_earlier_one("async", false, true, true)]
#[case::async_first_logger_redacts_the_later_one("async", true, false, true)]
#[case::async_every_logger_keeps_messages("async", false, false, false)]
#[case::sync_has_no_hook_loop("sync", false, true, false)]
#[tokio::test]
async fn logging_success_behavior_tests_behavior(
    #[case] path: &str,
    #[case] first_opted_out: bool,
    #[case] second_opted_out: bool,
    #[case] redacted: bool,
) {
    let mut fixture = Fixture::new();
    let calls = Arc::new(CallLog::default());
    fixture.add(&logger("langfuse", first_opted_out, &calls));
    fixture.add(&logger("datadog", second_opted_out, &calls));
    let mut state = state(Some(false));
    let mut call = request(SuccessResult::Chat(json_object(&[(
        "id",
        Json::Text("r1".to_owned()),
    )])));
    call.standard_logging_object = Some(secret_payload());
    let mut correlation = correlation();
    run_once(
        path,
        &fixture.handler(),
        &mut state,
        &mut correlation,
        &call,
    )
    .await;
    let (messages, response) = if redacted {
        (REDACTED_MESSAGES, REDACTED_RESPONSE)
    } else {
        (SECRET_MESSAGES, SECRET_RESPONSE)
    };
    let logged = calls.calls();
    assert_eq!(logged.len(), 2);
    assert!(logged.iter().all(|entry| {
        entry.messages == Some(parse(messages)) && entry.response == Some(parse(response))
    }));
    let stored = standard_logging_object(&state).unwrap().unwrap();
    assert_eq!(stored.messages, Some(parse(messages)));
}
