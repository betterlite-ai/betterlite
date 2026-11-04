#![cfg(test)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rstest::rstest;
use tracing::{Event, Subscriber, subscriber::with_default};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt as _},
};

use super::decode;

struct Warnings(Arc<AtomicUsize>);

impl<Inner> Layer<Inner> for Warnings
where
    Inner: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[rstest]
#[case::missing_keyword("blocked_words: [{action: MASK}]", 1)]
#[case::missing_action("blocked_words: [{keyword: x}]", 1)]
#[case::scalar_entry("blocked_words: [42]", 1)]
#[case::valid("blocked_words: [{keyword: x, action: MASK}]", 0)]
fn pattern_matching_behavior(#[case] yaml: &str, #[case] expected: usize) {
    let counter = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&counter)));
    with_default(subscriber, || decode(yaml.as_bytes()).unwrap());
    assert_eq!(counter.load(Ordering::Relaxed), expected);
}
