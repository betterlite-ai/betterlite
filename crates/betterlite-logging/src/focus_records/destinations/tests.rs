#![cfg(test)]

use super::{FocusDestination, FocusTimeWindow};
use chrono::DateTime;
use rstest::rstest;
use std::convert::Infallible;
use std::future::{Future, ready};
use std::pin::pin;
use std::task::{Context, Poll, Waker};

struct Destination;

impl FocusDestination for Destination {
    type Error = Infallible;

    fn deliver<'request>(
        &'request self,
        content: &'request [u8],
        time_window: &'request FocusTimeWindow,
        filename: &'request str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'request {
        assert_eq!(content, b"payload");
        assert!(time_window.start_time < time_window.end_time);
        assert_eq!(filename, "usage.parquet");
        ready(Ok(()))
    }
}

#[rstest]
#[case::hourly("hourly")]
#[case::daily("daily")]
#[case::interval("interval")]
#[case::all("all")]
#[case::custom("custom-frequency")]
fn accepts_export_window(#[case] frequency: &str) {
    let window = FocusTimeWindow {
        start_time: DateTime::from_timestamp(0, 123_456_000).unwrap(),
        end_time: DateTime::from_timestamp(3600, 123_456_000).unwrap(),
        frequency: frequency.to_owned(),
    };
    assert_eq!(window.frequency, frequency);
    assert_eq!(window.start_time.timestamp_subsec_micros(), 123_456);
    let mut delivery = pin!(Destination.deliver(b"payload", &window, "usage.parquet"));
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(delivery.as_mut().poll(&mut context), Poll::Ready(Ok(())));
}
