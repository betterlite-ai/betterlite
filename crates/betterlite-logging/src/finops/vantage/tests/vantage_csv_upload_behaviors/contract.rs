use std::sync::Arc;

use chrono::DateTime;

use crate::finops::vantage::tests::support::{FILENAME, FakeSender, destination, uploads, window};
use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};

const CONTENT: &[u8] = b"ServiceName\nCompute\n";

fn later_window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: DateTime::from_timestamp(7_200, 0).unwrap(),
        end_time: DateTime::from_timestamp(10_800, 0).unwrap(),
        frequency: "hourly".to_owned(),
    }
}

async fn uploaded_during(time_window: &FocusTimeWindow) -> Vec<(String, Vec<u8>)> {
    let sender = FakeSender::default();
    let posted = Arc::clone(&sender.posted);
    FocusDestination::deliver(&destination(sender), CONTENT, time_window, FILENAME)
        .await
        .unwrap();
    uploads(&posted.lock().unwrap())
}

#[tokio::test]
async fn the_export_window_does_not_change_the_upload() {
    let earlier = uploaded_during(&window()).await;
    let later = uploaded_during(&later_window()).await;
    assert_eq!(earlier, later);
    assert_eq!(earlier, vec![(FILENAME.to_owned(), CONTENT.to_vec())]);
}
