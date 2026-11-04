#![cfg(test)]

use super::*;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

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

#[test]
fn call_context_tests_behavior() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&events)));
    with_default(subscriber, || {
        assert_eq!(get_env_int("COUNT", None, 5), 5);
        assert_eq!(get_env_int("COUNT", Some("bad"), 5), 5);
    });
    let captured = events.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("COUNT"));
    drop(captured);

    assert_eq!(get_env_int("COUNT", Some(" 12 "), 5), 12);
    assert_eq!(get_env_int("COUNT", Some("bad"), 5), 5);
    assert_eq!(get_env_int_in_range("COUNT", Some("99"), 5, 0, 10), 5);
    assert_eq!(get_env_int_or_none(Some("")), None);
    assert_eq!(get_env_int_or_none(Some("7")), Some(7));
}
