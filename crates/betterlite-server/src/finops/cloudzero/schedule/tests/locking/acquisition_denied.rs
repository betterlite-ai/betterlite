use super::*;

#[tokio::test]
async fn cloudzero_schedule() {
    let lease = Arc::new(FakeLease::new(true, Some(false)));
    let assembled = logger(
        settings(),
        datastore(),
        FakeTransport::default(),
        Some(leased(&lease)),
    );
    initialize_export_job(&assembled)
        .await
        .expect("a skipped export");
    assert_eq!(lease.events(), vec![format!("acquire:{EXPORT_JOB_NAME}")]);
}
