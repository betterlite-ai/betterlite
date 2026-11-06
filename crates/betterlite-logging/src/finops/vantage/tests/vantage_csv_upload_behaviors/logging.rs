use rstest::rstest;
use tracing::subscriber::set_default;

use crate::finops::vantage::tests::support::{
    Captured, FILENAME, FakeSender, HEADER, csv, deliver, rows,
};

fn filtered_content() -> Vec<u8> {
    b"ChargeCategory,ProviderName,BilledCost\nUsage,x,1\n".to_vec()
}

fn oversized_row_content() -> Vec<u8> {
    csv(HEADER, &["x".repeat(2 * 1024 * 1024)])
}

fn failing_first_batch() -> FakeSender {
    FakeSender::failing_on(vec![format!("{FILENAME}.part0")])
}

fn failing_first_sub_batch() -> FakeSender {
    FakeSender::failing_on(vec![format!("{FILENAME}.part0_0")])
}

#[rstest]
#[case::empty_content(
    Vec::new(),
    FakeSender::default(),
    "DEBUG",
    "empty content, skipping upload"
)]
#[case::dropped_columns(
    filtered_content(),
    FakeSender::default(),
    "DEBUG",
    "dropping unsupported columns"
)]
#[case::dropped_column_names(filtered_content(), FakeSender::default(), "DEBUG", "ProviderName")]
#[case::upload_size_and_name(
    filtered_content(),
    FakeSender::default(),
    "DEBUG",
    "uploaded 36 bytes"
)]
#[case::oversized_row(
    oversized_row_content(),
    FakeSender::default(),
    "WARN",
    "skipping oversized row"
)]
#[case::oversized_row_size(oversized_row_content(), FakeSender::default(), "WARN", "2097153")]
#[case::failed_batch(csv(HEADER, &rows(20_000, 8)), failing_first_batch(), "ERROR", "batch 0 failed")]
#[case::failed_sub_batch(csv(HEADER, &vec!["x".repeat(700_000); 3]), failing_first_sub_batch(), "ERROR", "sub-batch usage.csv.part0_0 failed")]
#[tokio::test]
async fn deliveries_log_what_they_did(
    #[case] content: Vec<u8>,
    #[case] sender: FakeSender,
    #[case] level: &str,
    #[case] fragment: &str,
) {
    let captured = Captured::default();
    let _guard = set_default(captured.subscriber());
    drop(deliver(sender, &content).await);
    let output = captured.text();
    let logged = output
        .lines()
        .any(|line| line.contains(level) && line.contains(fragment));
    assert!(logged, "{level} {fragment} missing from:\n{output}");
}
