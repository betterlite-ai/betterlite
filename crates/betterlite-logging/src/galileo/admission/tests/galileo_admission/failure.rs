use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::{CapturedLogs, kwargs, standard_object, user_messages};
use crate::galileo::admission::async_log_failure_event;

#[test]
fn failure_callback_is_diagnostic_only() {
    let call_kwargs = kwargs(Some(standard_object()), Some(user_messages("hi")));
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());

    with_default(subscriber, || async_log_failure_event(&call_kwargs));

    assert_eq!(logs.matching(Level::DEBUG, "On Async Failure").len(), 1);
}
