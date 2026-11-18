#![cfg(test)]

use std::sync::Arc;

use rust_decimal::Decimal;

use super::fixtures::{
    FakeLease, FakeScheduler, FakeTransport, FixedClock, clone_store, close, database, datastore,
    insert_usage, lease_events, leased, logger, minutes_ago, settings, settings_from, usage,
};
use crate::finops::cloudzero::schedule::CloudZeroLogger;
use crate::finops::cloudzero::schedule::EXPORT_JOB_NAME;
use crate::finops::cloudzero::schedule::background::init_cloudzero_background_job;

mod registration {
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
}

mod no_loggers {
    use super::*;

    #[test]
    fn cloudzero_schedule() {
        let mut scheduler = FakeScheduler::default();
        let loggers: Vec<Arc<CloudZeroLogger<FakeTransport, FixedClock>>> = Vec::new();
        init_cloudzero_background_job(&mut scheduler, &loggers);
        assert!(scheduler.jobs.is_empty());
    }
}

mod job_body {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
        )
        .await;
        let transport = FakeTransport::default();
        let lease = Arc::new(FakeLease::new(true, Some(true)));
        let assembled = Arc::new(logger(
            settings(),
            clone_store(&store),
            transport.clone(),
            leased(&lease),
        ));
        let mut scheduler = FakeScheduler::default();
        init_cloudzero_background_job(&mut scheduler, &[assembled]);
        scheduler.jobs[0].2.run().await;
        assert_eq!(transport.posts().len(), 1);
        assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
        close(fixture, &store).await;
    }
}

mod job_failure {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
        )
        .await;
        let transport = FakeTransport::failing();
        let lease = Arc::new(FakeLease::new(true, Some(true)));
        let assembled = Arc::new(logger(
            settings(),
            clone_store(&store),
            transport.clone(),
            leased(&lease),
        ));
        let mut scheduler = FakeScheduler::default();
        init_cloudzero_background_job(&mut scheduler, &[assembled]);
        scheduler.jobs[0].2.run().await;
        assert_eq!(transport.posts().len(), 1);
        assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
        close(fixture, &store).await;
    }
}
