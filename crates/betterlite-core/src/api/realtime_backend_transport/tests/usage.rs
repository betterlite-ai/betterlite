use super::harness::{HarnessOptions, harness};
use crate::api::realtime_contracts::events::AudioTranscriptionUsage;

#[tokio::test]
async fn realtime_backend_flushes_the_provider_usage_on_close() {
    let usage: AudioTranscriptionUsage =
        serde_json::from_str("{\"type\":\"duration\",\"seconds\":4}").unwrap();
    let options = HarnessOptions::default().with_close_usage(Some(usage));
    let mut billed = harness(options, Vec::new());
    billed.run().await.unwrap();
    assert_eq!(billed.observation.lock().unwrap().stored_messages, 1);
}
