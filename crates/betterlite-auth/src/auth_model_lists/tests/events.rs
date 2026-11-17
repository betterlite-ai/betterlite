use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

struct EventCapture {
    warnings: Arc<AtomicUsize>,
    errors: Arc<AtomicUsize>,
}

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        match *event.metadata().level() {
            tracing::Level::WARN => {
                self.warnings.fetch_add(1, Ordering::Relaxed);
            }
            tracing::Level::ERROR => {
                self.errors.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

pub(super) fn captured<T, F>(run: F) -> (T, usize, usize)
where
    F: FnOnce() -> T,
{
    let warnings = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(AtomicUsize::new(0));
    let capture = EventCapture {
        warnings: Arc::clone(&warnings),
        errors: Arc::clone(&errors),
    };
    let subscriber = tracing_subscriber::registry().with(capture);
    let value = with_default(subscriber, run);
    (
        value,
        warnings.load(Ordering::Relaxed),
        errors.load(Ordering::Relaxed),
    )
}
