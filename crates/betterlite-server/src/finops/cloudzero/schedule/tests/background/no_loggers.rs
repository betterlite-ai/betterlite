use super::*;

#[test]
fn cloudzero_schedule() {
    let mut scheduler = FakeScheduler::default();
    let loggers: Vec<Arc<CloudZeroLogger<FakeTransport, FixedClock>>> = Vec::new();
    init_cloudzero_background_job(&mut scheduler, &loggers);
    assert!(scheduler.jobs.is_empty());
}
