#![cfg(test)]

use std::sync::Arc;

use rust_decimal::Decimal;

use super::fixtures::{
    FakeLease, FakeTransport, clone_store, close, database, datastore, insert_usage, lease_events,
    leased, logger, minutes_ago, settings, usage,
};
use crate::finops::cloudzero::schedule::EXPORT_JOB_NAME;
use crate::finops::cloudzero::schedule::error::CloudZeroExportError;
use crate::finops::cloudzero::schedule::export::initialize_export_job;

mod acquired {
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
        let assembled = logger(
            settings(),
            clone_store(&store),
            transport.clone(),
            leased(&lease),
        );
        initialize_export_job(&assembled)
            .await
            .expect("a locked export");
        assert_eq!(transport.posts().len(), 1);
        assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
        close(fixture, &store).await;
    }
}

mod acquisition_denied {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let lease = Arc::new(FakeLease::new(true, Some(false)));
        let assembled = logger(
            settings(),
            datastore(),
            FakeTransport::default(),
            leased(&lease),
        );
        initialize_export_job(&assembled)
            .await
            .expect("a skipped export");
        assert_eq!(lease.events(), vec![format!("acquire:{EXPORT_JOB_NAME}")]);
    }
}

mod acquisition_unknown {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let lease = Arc::new(FakeLease::new(true, None));
        let assembled = logger(
            settings(),
            datastore(),
            FakeTransport::default(),
            leased(&lease),
        );
        initialize_export_job(&assembled)
            .await
            .expect("a skipped export");
        assert_eq!(lease.events(), vec![format!("acquire:{EXPORT_JOB_NAME}")]);
    }
}

mod direct_export {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-1", "gpt-4o", Decimal::ONE, minutes_ago(10)),
        )
        .await;
        let unavailable_transport = FakeTransport::default();
        let unavailable = Arc::new(FakeLease::new(false, Some(false)));
        let unavailable_logger = logger(
            settings(),
            clone_store(&store),
            unavailable_transport.clone(),
            leased(&unavailable),
        );
        initialize_export_job(&unavailable_logger)
            .await
            .expect("a direct export");
        assert_eq!(unavailable_transport.posts().len(), 1);
        assert!(unavailable.events().is_empty());
        let unlocked_transport = FakeTransport::default();
        let unlocked_logger = logger(
            settings(),
            clone_store(&store),
            unlocked_transport.clone(),
            None,
        );
        initialize_export_job(&unlocked_logger)
            .await
            .expect("an unlocked export");
        assert_eq!(unlocked_transport.posts().len(), 1);
        close(fixture, &store).await;
    }
}

mod release_on_failure {
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
        let assembled = logger(
            settings(),
            clone_store(&store),
            transport.clone(),
            leased(&lease),
        );
        let error = initialize_export_job(&assembled)
            .await
            .expect_err("the post is refused");
        assert!(matches!(error, CloudZeroExportError::Delivery(_)));
        assert_eq!(transport.posts().len(), 1);
        assert_eq!(lease.events(), lease_events(EXPORT_JOB_NAME));
        close(fixture, &store).await;
    }
}
