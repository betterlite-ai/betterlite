#![cfg(test)]

use rust_decimal::Decimal;

use super::fixtures::{
    FakeTransport, clone_store, close, database, datastore, insert_usage_specs, logger,
    minutes_ago, settings, settings_from, usage,
};
use crate::finops::cloudzero::schedule::error::CloudZeroExportError;
use crate::finops::cloudzero::schedule::export::hourly_usage_data_export;

mod default_window {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage_specs(
            &store,
            &[
                usage("row-121", "model-a", Decimal::ONE, minutes_ago(121)),
                usage("row-120", "model-b", Decimal::ONE, minutes_ago(120)),
                usage("row-119", "model-c", Decimal::ONE, minutes_ago(119)),
            ],
        )
        .await;
        let transport = FakeTransport::default();
        let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
        hourly_usage_data_export(&assembled)
            .await
            .expect("the hourly export");
        let posts = transport.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].operation, "replace_hourly");
        assert_eq!(posts[0].data.len(), 2);
        close(fixture, &store).await;
    }
}

mod custom_interval {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage_specs(
            &store,
            &[
                usage("row-61", "model-a", Decimal::ONE, minutes_ago(61)),
                usage("row-60", "model-b", Decimal::ONE, minutes_ago(60)),
                usage("row-59", "model-c", Decimal::ONE, minutes_ago(59)),
                usage("row-31", "model-d", Decimal::ONE, minutes_ago(31)),
            ],
        )
        .await;
        let transport = FakeTransport::default();
        let settings = settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "30")]);
        let assembled = logger(settings, clone_store(&store), transport.clone(), None);
        hourly_usage_data_export(&assembled)
            .await
            .expect("the custom export");
        assert_eq!(transport.posts()[0].data.len(), 3);
        close(fixture, &store).await;
    }
}

mod record_cap {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage_specs(
            &store,
            &[
                usage("row-10", "model-a", Decimal::ONE, minutes_ago(10)),
                usage("row-11", "model-b", Decimal::ONE, minutes_ago(11)),
                usage("row-12", "model-c", Decimal::ONE, minutes_ago(12)),
            ],
        )
        .await;
        let transport = FakeTransport::default();
        let settings = settings_from(&[("CLOUDZERO_MAX_FETCHED_DATA_RECORDS", "1")]);
        let assembled = logger(settings, clone_store(&store), transport.clone(), None);
        hourly_usage_data_export(&assembled)
            .await
            .expect("the capped export");
        assert_eq!(transport.posts()[0].data.len(), 1);
        close(fixture, &store).await;
    }
}

mod out_of_range {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let doubled =
            settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "9223372036854775807")]);
        let doubled_logger = logger(doubled, datastore(), FakeTransport::default(), None);
        let error = hourly_usage_data_export(&doubled_logger)
            .await
            .expect_err("a doubled interval overflows");
        assert!(matches!(error, CloudZeroExportError::WindowOutOfRange));
        assert_eq!(
            error.to_string(),
            "the export window is outside the supported date range"
        );
        let too_long =
            settings_from(&[("CLOUDZERO_EXPORT_INTERVAL_MINUTES", "4611686018427387903")]);
        let too_long_logger = logger(too_long, datastore(), FakeTransport::default(), None);
        let error = hourly_usage_data_export(&too_long_logger)
            .await
            .expect_err("the lookback exceeds the duration range");
        assert!(matches!(error, CloudZeroExportError::WindowOutOfRange));
    }
}
