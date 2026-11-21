use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::unfold;
use rstest::rstest;
use tokio::time::sleep;

use crate::agent_endpoints::a2a_stream::FrameStream;
use crate::agent_endpoints::a2a_stream::error::StreamAbort;
use crate::agent_endpoints::a2a_stream::keepalive::{PING, coerce_keepalive_interval, with_pings};

#[rstest]
#[case::unset(None, None)]
#[case::zero(Some(0.0_f64), None)]
#[case::negative_zero(Some(-0.0_f64), None)]
#[case::negative(Some(-2.5_f64), None)]
#[case::not_a_number(Some(f64::NAN), None)]
#[case::infinite(Some(f64::INFINITY), None)]
#[case::negative_infinite(Some(f64::NEG_INFINITY), None)]
#[case::too_large(Some(f64::MAX), None)]
#[case::half_second(Some(0.5_f64), Some(Duration::from_millis(500)))]
#[case::one_nanosecond(Some(1e-9_f64), Some(Duration::from_nanos(1)))]
fn coerce_keepalive_interval_takes_finite_positive_seconds(
    #[case] seconds: Option<f64>,
    #[case] interval: Option<Duration>,
) {
    assert_eq!(coerce_keepalive_interval(seconds), interval);
}

// each frame comes after its delay, measured from the frame before
fn delayed(frames: Vec<(u64, Result<String, StreamAbort>)>) -> FrameStream {
    unfold(frames.into_iter(), |mut rest| async move {
        let (delay, frame) = rest.next()?;
        sleep(Duration::from_millis(delay)).await;
        Some((frame, rest))
    })
    .boxed()
}

#[tokio::test(start_paused = true)]
async fn with_pings_fills_every_idle_gap() {
    let frames = delayed(vec![
        (2500, Ok(String::from("data: 1\n\n"))),
        (1500, Ok(String::from("data: 2\n\n"))),
        (500, Ok(String::from("data: 3\n\n"))),
    ]);
    let body: Vec<String> = with_pings(frames, Duration::from_secs(1))
        .map(Result::unwrap)
        .collect()
        .await;
    assert_eq!(
        body,
        [
            PING,
            PING,
            "data: 1\n\n",
            PING,
            "data: 2\n\n",
            "data: 3\n\n"
        ]
    );
    assert_eq!(PING, ": ping\n\n");
}

#[tokio::test(start_paused = true)]
async fn with_pings_passes_an_abort_through_in_order() {
    let abort = StreamAbort {
        message: String::from("cut"),
    };
    let frames = delayed(vec![(0, Ok(String::from("data: 1\n\n"))), (0, Err(abort))]);
    let body: Vec<Result<String, StreamAbort>> =
        with_pings(frames, Duration::from_secs(1)).collect().await;
    assert_eq!(
        body,
        [
            Ok(String::from("data: 1\n\n")),
            Err(StreamAbort {
                message: String::from("cut")
            })
        ]
    );
}
