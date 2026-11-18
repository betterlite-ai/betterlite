#![cfg(test)]

mod ack_cases;
mod completion_cases;
mod corpus;
mod event_dispatch;
mod failure_cases;
mod limits;
mod logging_cases;
mod session_cases;
mod stream_cases;
mod support;
mod transport;
mod usage_cases;

#[derive(Clone, Copy, Debug)]
enum EventCase {
    NativeEventDispatch,
    SessionUpdateFirst,
    PendingSessionUpdate,
    ProviderFailureEvent,
    NativeSessionAndState,
    NativeUserTranscriptDispatch,
    NativeCompletionDispatch,
    AckSessionUpdateOrder,
    EventLoggingSelection,
}

#[rstest::rstest]
#[case::native_event_dispatch(EventCase::NativeEventDispatch)]
#[case::session_update_first(EventCase::SessionUpdateFirst)]
#[case::pending_session_update(EventCase::PendingSessionUpdate)]
#[case::provider_failure_event(EventCase::ProviderFailureEvent)]
#[case::native_session_and_state(EventCase::NativeSessionAndState)]
#[case::native_user_transcript_dispatch(EventCase::NativeUserTranscriptDispatch)]
#[case::native_completion_dispatch(EventCase::NativeCompletionDispatch)]
#[case::ack_session_update_order(EventCase::AckSessionUpdateOrder)]
#[case::event_logging_selection(EventCase::EventLoggingSelection)]
#[tokio::test]
async fn realtime_events_cases(#[case] case: EventCase) {
    match case {
        EventCase::NativeEventDispatch => event_dispatch::native_event_dispatch(),
        EventCase::SessionUpdateFirst => stream_cases::session_update_first().await,
        EventCase::PendingSessionUpdate => stream_cases::pending_session_update(),
        EventCase::ProviderFailureEvent => failure_cases::provider_failure_event(),
        EventCase::NativeSessionAndState => session_cases::native_session_and_state(),
        EventCase::NativeUserTranscriptDispatch => session_cases::native_user_transcript_dispatch(),
        EventCase::NativeCompletionDispatch => completion_cases::native_completion_dispatch(),
        EventCase::AckSessionUpdateOrder => ack_cases::ack_session_update_order().await,
        EventCase::EventLoggingSelection => logging_cases::event_logging_selection().await,
    }
}
