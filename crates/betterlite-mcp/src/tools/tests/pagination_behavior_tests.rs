#![cfg(test)]

use super::{fake, timeouts, tool};
use crate::tools::pagination::{MAX_PAGES, Page, ToolError, list_tools_with_pagination};
use rstest::rstest;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{Subscriber, subscriber::set_default};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

struct Warnings(Arc<Mutex<usize>>);
impl<Current> Layer<Current> for Warnings
where
    Current: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Current>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut count = self.0.lock().unwrap();
            *count = count.saturating_add(1);
        }
    }
}

#[rstest]
#[case::complete(None, 0)]
#[case::empty_cursor(Some(""), 0)]
#[case::repeated_cursor(Some("next"), 1)]
#[tokio::test(start_paused = true)]
async fn follows_pages_and_keeps_partial_catalog(
    #[case] end: Option<&str>,
    #[case] warnings: usize,
) {
    let count = Arc::new(Mutex::new(0_usize));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&count)));
    let _guard = set_default(subscriber);
    let mut session = fake(vec![
        (
            Duration::ZERO,
            Page {
                tools: vec![tool()],
                next_cursor: Some("next".to_owned()),
            },
        ),
        (
            Duration::ZERO,
            Page {
                tools: vec![tool()],
                next_cursor: end.map(str::to_owned),
            },
        ),
    ]);
    let result = list_tools_with_pagination(&mut session, timeouts(60, 30))
        .await
        .unwrap();
    assert_eq!(result, vec![tool(), tool()]);
    assert_eq!(session.cursors, vec![None, Some("next".to_owned())]);
    assert_eq!(*count.lock().unwrap(), warnings);
}

#[tokio::test(start_paused = true)]
async fn page_exhaustion_warns_and_keeps_every_tool() {
    let count = Arc::new(Mutex::new(0_usize));
    let _guard = set_default(tracing_subscriber::registry().with(Warnings(Arc::clone(&count))));
    assert_eq!(MAX_PAGES, 1_000_usize);
    let pages = (0..=MAX_PAGES)
        .map(|index| {
            (
                Duration::ZERO,
                Page {
                    tools: vec![tool()],
                    next_cursor: Some(index.to_string()),
                },
            )
        })
        .collect();
    let mut session = fake(pages);
    assert_eq!(
        list_tools_with_pagination(&mut session, timeouts(60, 30))
            .await
            .unwrap()
            .len(),
        MAX_PAGES
    );
    assert_eq!(session.cursors.len(), MAX_PAGES);
    assert_eq!(*count.lock().unwrap(), 1);
}

#[rstest]
#[case::client_deadline((60, 30, None), 40, 1, 0)]
#[case::listing_deadline((30, 60, None), 40, 1, 0)]
#[case::override_extends((10, 20, Some(90)), 70, 1, 0)]
#[case::override_shortens((60, 30, Some(10)), 40, 0, 1)]
#[case::expired((10, 20, None), 40, 0, 1)]
#[case::zero((0, 0, None), 0, 0, 1)]
#[tokio::test(start_paused = true)]
async fn deadline_and_single_slow_page(
    #[case] config: (u64, u64, Option<u64>),
    #[case] delay: u64,
    #[case] tools: usize,
    #[case] warnings: usize,
) {
    let count = Arc::new(Mutex::new(0_usize));
    let _guard = set_default(tracing_subscriber::registry().with(Warnings(Arc::clone(&count))));
    let mut session = fake(vec![(
        Duration::from_secs(delay),
        Page {
            tools: vec![tool()],
            next_cursor: None,
        },
    )]);
    let (client, listing, deadline) = config;
    let mut limits = timeouts(client, listing);
    limits.listing_deadline = deadline.map(Duration::from_secs);
    assert_eq!(
        list_tools_with_pagination(&mut session, limits)
            .await
            .unwrap()
            .len(),
        tools
    );
    assert_eq!(*count.lock().unwrap(), warnings);
}

#[tokio::test(start_paused = true)]
async fn deadline_is_for_the_whole_walk() {
    let count = Arc::new(Mutex::new(0_usize));
    let _guard = set_default(tracing_subscriber::registry().with(Warnings(Arc::clone(&count))));
    let mut session = fake(vec![
        (
            Duration::from_secs(40),
            Page {
                tools: vec![tool()],
                next_cursor: Some("next".to_owned()),
            },
        ),
        (
            Duration::from_secs(40),
            Page {
                tools: vec![tool()],
                next_cursor: None,
            },
        ),
    ]);
    assert_eq!(
        list_tools_with_pagination(&mut session, timeouts(60, 30))
            .await
            .unwrap(),
        vec![tool()]
    );
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn session_errors_keep_their_source() {
    let error = list_tools_with_pagination(&mut fake(Vec::new()), timeouts(60, 30))
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::Session(source) if source.to_string() == "missing page"));
}
