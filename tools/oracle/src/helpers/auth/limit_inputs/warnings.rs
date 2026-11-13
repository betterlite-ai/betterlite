use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

#[derive(Clone, Default)]
pub(crate) struct WarningLog(Arc<Mutex<Vec<String>>>);

impl WarningLog {
    pub(crate) fn warnings(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(warnings) => warnings.clone(),
            Err(_) => Vec::new(),
        }
    }
}

impl<S: Subscriber> Layer<S> for WarningLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut visitor = WarningFields::default();
        event.record(&mut visitor);
        if let Some(field) = visitor.0
            && let Ok(mut warnings) = self.0.lock()
        {
            warnings.push(field);
        }
    }
}

#[derive(Default)]
struct WarningFields(Option<String>);

impl Visit for WarningFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "field" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}
