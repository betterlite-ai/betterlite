use super::*;

#[test]
fn cloudzero_schedule() {
    let mut configured = FakeScheduler::default();
    let configured_settings = settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "30")]);
    let configured_logger = Arc::new(logger(
        configured_settings,
        datastore(),
        FakeTransport::default(),
        None,
    ));
    init_cloudzero_background_job(&mut configured, &[configured_logger]);
    assert_eq!(configured.jobs.len(), 1);
    assert_eq!(configured.jobs[0].0, EXPORT_JOB_NAME);
    assert_eq!(configured.jobs[0].1, 30);
    let mut defaulted = FakeScheduler::default();
    let defaulted_logger = Arc::new(logger(
        settings(),
        datastore(),
        FakeTransport::default(),
        None,
    ));
    init_cloudzero_background_job(&mut defaulted, &[defaulted_logger]);
    assert_eq!(defaulted.jobs[0].1, 60);
}
