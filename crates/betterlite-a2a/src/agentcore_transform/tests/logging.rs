use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::agentcore_transform::headers::filter_reserved_headers;

struct WarningCount(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for WarningCount {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() == Level::WARN {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[rstest]
#[case::reserved_header("Authorization", 1)]
#[case::safe_header("x-request-id", 0)]
fn the_filter_warns_once_when_it_drops_a_header(#[case] name: &str, #[case] warnings: usize) {
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(WarningCount(Arc::clone(&count)));
    let headers = BTreeMap::from([(name.to_owned(), "value".to_owned())]);

    with_default(subscriber, || filter_reserved_headers(&headers));

    assert_eq!(count.load(Ordering::SeqCst), warnings);
}
