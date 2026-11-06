use rstest::rstest;

use crate::finops::vantage::error::DeliveryError;
use crate::finops::vantage::tests::support::{FILENAME, FakeSender, HEADER, csv, deliver, rows};

fn named(suffixes: &[&str]) -> Vec<String> {
    suffixes
        .iter()
        .map(|suffix| format!("{FILENAME}{suffix}"))
        .collect()
}

fn three_wide_rows() -> Vec<String> {
    vec!["x".repeat(700_000); 3]
}

#[rstest]
#[case::first_of_two_batches(20_000, 8, &[".part0"], ".part0", &[".part0", ".part1"])]
#[case::both_batches_report_the_first(20_000, 8, &[".part0", ".part1"], ".part0", &[".part0", ".part1"])]
#[case::only_the_last_of_three(25_000, 8, &[".part2"], ".part2", &[".part0", ".part1", ".part2"])]
#[case::middle_batch_with_the_rest_attempted(30_000, 8, &[".part1"], ".part1", &[".part0", ".part1", ".part2"])]
#[case::first_sub_batch(10_001, 250, &[".part0_0"], ".part0_0", &[".part0_0", ".part0_1", ".part1"])]
#[case::second_sub_batch(10_001, 250, &[".part0_1"], ".part0_1", &[".part0_0", ".part0_1", ".part1"])]
#[case::sub_batch_error_beats_a_later_batch(10_001, 250, &[".part0_1", ".part1"], ".part0_1", &[".part0_0", ".part0_1", ".part1"])]
#[tokio::test]
async fn the_first_error_is_raised_after_every_batch_was_attempted(
    #[case] count: usize,
    #[case] width: usize,
    #[case] failing: &[&str],
    #[case] first: &str,
    #[case] attempted: &[&str],
) {
    let sender = FakeSender::failing_on(named(failing));
    let (result, posted) = deliver(sender, &csv(HEADER, &rows(count, width))).await;
    let Err(DeliveryError::Send(error)) = result else {
        panic!("the failed uploads were not reported: {result:?}");
    };
    assert_eq!(error.0, format!("{FILENAME}{first}"));
    let names: Vec<String> = posted.into_iter().map(|post| post.filename).collect();
    assert_eq!(names, named(attempted));
}

#[rstest]
#[case::first_sub_batch(&[".part0_0"], ".part0_0")]
#[case::last_sub_batch(&[".part0_1"], ".part0_1")]
#[tokio::test]
async fn a_failed_size_chunk_does_not_stop_the_next_one(
    #[case] failing: &[&str],
    #[case] first: &str,
) {
    let sender = FakeSender::failing_on(named(failing));
    let (result, posted) = deliver(sender, &csv(HEADER, &three_wide_rows())).await;
    let Err(DeliveryError::Send(error)) = result else {
        panic!("the failed upload was not reported: {result:?}");
    };
    assert_eq!(error.0, format!("{FILENAME}{first}"));
    assert_eq!(posted.len(), 2);
}

#[rstest]
#[case::ok(200, true)]
#[case::no_content(204, true)]
#[case::redirect(302, false)]
#[case::client_error(422, false)]
#[case::server_error(500, false)]
#[tokio::test]
async fn only_a_success_status_counts_as_uploaded(#[case] status: u16, #[case] uploaded: bool) {
    let sender = FakeSender::answering(status);
    let (result, _) = deliver(sender, &csv(HEADER, &rows(2, 8))).await;
    match result {
        Ok(()) => assert!(uploaded),
        Err(DeliveryError::Status { status: reported }) => {
            assert!(!uploaded);
            assert_eq!(reported, status);
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn a_bad_status_on_one_batch_still_lets_the_others_upload() {
    let sender = FakeSender::answering(503);
    let (result, posted) = deliver(sender, &csv(HEADER, &rows(20_000, 8))).await;
    assert!(matches!(result, Err(DeliveryError::Status { status: 503 })));
    assert_eq!(posted.len(), 2);
}

#[tokio::test]
async fn undecodable_content_is_refused_before_any_upload() {
    let (result, posted) = deliver(FakeSender::default(), b"Junk,ServiceName\n\xff,1\n").await;
    assert!(matches!(result, Err(DeliveryError::Columns(_))));
    assert!(posted.is_empty());
}
