#![cfg(test)]

use super::{get_env_int, get_env_int_in_range, get_env_int_or_none};
use rstest::rstest;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

#[derive(Clone)]
struct EventCapture(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        let mut fields = FieldCapture::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.push(fields.0.join(" "));
        }
    }
}

#[derive(Default)]
struct FieldCapture(Vec<String>);

impl Visit for FieldCapture {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

#[rstest]
#[case::absent(None, 5, None, 0)]
#[case::invalid(Some("bad"), 5, None, 0)]
#[case::empty(Some(""), 5, None, 0)]
#[case::whitespace(Some(" 7 "), 7, Some(7), 0)]
#[case::minimum(Some("0"), 0, Some(0), 0)]
#[case::maximum(Some("10"), 10, Some(10), 0)]
#[case::above(Some("11"), 5, Some(11), 1)]
#[case::below(Some("-1"), 5, Some(-1), 1)]
#[case::overflow(Some("9223372036854775808"), 5, None, 0)]
fn call_context_tests_behavior(
    #[case] value: Option<&str>,
    #[case] ranged: i64,
    #[case] optional: Option<i64>,
    #[case] warnings: usize,
) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    with_default(subscriber, || {
        assert_eq!(get_env_int_in_range("COUNT", value, 5, 0, 10), ranged);
    });
    let captured = events.lock().unwrap();
    assert_eq!(captured.len(), warnings);
    assert!(captured.iter().all(|event| event.contains("COUNT")));
    drop(captured);
    assert_eq!(get_env_int(value, 5), optional.unwrap_or(5));
    assert_eq!(get_env_int_or_none(value), optional);
}
