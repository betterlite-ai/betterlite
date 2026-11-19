use std::sync::{Arc, Mutex, PoisonError};

pub(crate) type Trace = Arc<Mutex<Vec<String>>>;

pub(crate) fn record(trace: &Trace, event: &str) {
    trace
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event.to_owned());
}
