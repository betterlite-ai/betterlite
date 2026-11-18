#![cfg(test)]

use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::fixtures::{
    FakeTransport, clone_store, close, database, insert_generated_usage, insert_usage, logger,
    minutes_ago, settings, usage,
};
use crate::finops::cloudzero::schedule::CloudZeroSettings;
use crate::finops::cloudzero::schedule::dry_run::dry_run_export_usage_data;

mod empty {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        let assembled = logger(
            settings(),
            clone_store(&store),
            FakeTransport::default(),
            None,
        );
        let outcome = dry_run_export_usage_data(&assembled, None)
            .await
            .expect("an empty dry run");
        assert!(outcome.usage_data.is_empty());
        assert!(outcome.cbf_data.is_empty());
        assert_eq!(outcome.summary.total_records, 0);
        assert_eq!(outcome.summary.total_cost, Decimal::ZERO);
        assert_eq!(outcome.summary.total_tokens, 0);
        assert_eq!(outcome.summary.unique_accounts, 0);
        assert_eq!(outcome.summary.unique_services, 0);
        close(fixture, &store).await;
    }
}

mod sample_and_summary {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_generated_usage(&store, minutes_ago(10), 60).await;
        let assembled = logger(
            settings(),
            clone_store(&store),
            FakeTransport::default(),
            None,
        );
        let full = dry_run_export_usage_data(&assembled, None)
            .await
            .expect("a full dry run");
        assert_eq!(full.usage_data.len(), 50);
        assert_eq!(full.cbf_data.len(), 60);
        assert_eq!(full.summary.total_records, 60);
        assert_eq!(full.summary.total_cost, Decimal::new(90, 0));
        assert_eq!(full.summary.total_tokens, 180);
        assert_eq!(full.summary.unique_accounts, 1);
        assert_eq!(full.summary.unique_services, 1);
        let limited = dry_run_export_usage_data(&assembled, Some(3))
            .await
            .expect("a limited dry run");
        assert_eq!(limited.usage_data.len(), 3);
        assert_eq!(limited.cbf_data.len(), 3);
        assert_eq!(limited.summary.total_records, 3);
        assert_eq!(limited.summary.total_cost, Decimal::new(45, 1));
        assert_eq!(limited.summary.total_tokens, 9);
        close(fixture, &store).await;
    }
}

mod fallback {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        let mut first = usage("row-low-1", "model-a", Decimal::new(25, 1), minutes_ago(5));
        first.successful_requests = 0;
        insert_usage(&store, &first).await;
        let mut second = usage("row-low-2", "model-b", Decimal::new(35, 1), minutes_ago(5));
        second.successful_requests = 0;
        insert_usage(&store, &second).await;
        let empty: BTreeMap<String, String> = BTreeMap::new();
        let unconfigured = CloudZeroSettings::resolve(None, None, None, &empty).expect("settings");
        let assembled = logger(
            unconfigured,
            clone_store(&store),
            FakeTransport::default(),
            None,
        );
        let outcome = dry_run_export_usage_data(&assembled, None)
            .await
            .expect("a fallback dry run");
        assert_eq!(outcome.usage_data.len(), 2);
        assert!(outcome.cbf_data.is_empty());
        assert_eq!(outcome.summary.total_records, 2);
        assert_eq!(outcome.summary.total_cost, Decimal::new(6, 0));
        assert_eq!(outcome.summary.total_tokens, 6);
        assert_eq!(outcome.summary.unique_accounts, 0);
        assert_eq!(outcome.summary.unique_services, 0);
        close(fixture, &store).await;
    }
}
