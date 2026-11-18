#![cfg(test)]

use rust_decimal::Decimal;

use super::fixtures::{
    FakeTransport, clone_store, close, database, insert_usage, logger, minutes_ago, now, settings,
    usage,
};
use crate::finops::cloudzero::schedule::ExportOperation;
use crate::finops::cloudzero::schedule::error::CloudZeroExportError;
use crate::finops::cloudzero::schedule::export::export_usage_data;

mod operations {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-1", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
        )
        .await;
        let default_transport = FakeTransport::default();
        let default_logger = logger(
            settings(),
            clone_store(&store),
            default_transport.clone(),
            None,
        );
        export_usage_data(&default_logger, None, None, None, None)
            .await
            .expect("a default export");
        let posts = default_transport.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].operation, "replace_hourly");
        assert_eq!(posts[0].month, "2026-01");
        assert_eq!(posts[0].data.len(), 1);
        assert_eq!(
            posts[0].data[0].get("cost/cost").map(String::as_str),
            Some("1.5")
        );
        let sum_transport = FakeTransport::default();
        let sum_logger = logger(settings(), clone_store(&store), sum_transport.clone(), None);
        export_usage_data(&sum_logger, None, Some(ExportOperation::Sum), None, None)
            .await
            .expect("a sum export");
        assert_eq!(sum_transport.posts()[0].operation, "sum");
        close(fixture, &store).await;
    }
}

mod limit {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-new", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
        )
        .await;
        insert_usage(
            &store,
            &usage(
                "row-mid",
                "gpt-4o-mini",
                Decimal::new(25, 1),
                minutes_ago(30),
            ),
        )
        .await;
        insert_usage(
            &store,
            &usage("row-old", "gpt-4", Decimal::new(35, 1), minutes_ago(200)),
        )
        .await;
        let transport = FakeTransport::default();
        let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
        export_usage_data(&assembled, Some(1), None, None, None)
            .await
            .expect("a limited export");
        assert_eq!(transport.posts()[0].data.len(), 1);
        close(fixture, &store).await;
    }
}

mod window {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-new", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
        )
        .await;
        insert_usage(
            &store,
            &usage(
                "row-mid",
                "gpt-4o-mini",
                Decimal::new(25, 1),
                minutes_ago(30),
            ),
        )
        .await;
        insert_usage(
            &store,
            &usage("row-old", "gpt-4", Decimal::new(35, 1), minutes_ago(200)),
        )
        .await;
        let transport = FakeTransport::default();
        let assembled = logger(settings(), clone_store(&store), transport.clone(), None);
        export_usage_data(
            &assembled,
            None,
            None,
            Some(minutes_ago(60).and_utc()),
            Some(now()),
        )
        .await
        .expect("a windowed export");
        assert_eq!(transport.posts()[0].data.len(), 2);
        close(fixture, &store).await;
    }
}

mod empty_and_failure {
    use super::*;

    #[tokio::test]
    async fn cloudzero_schedule() {
        let (fixture, store) = database().await;
        insert_usage(
            &store,
            &usage("row-1", "gpt-4o", Decimal::new(15, 1), minutes_ago(10)),
        )
        .await;
        let empty_transport = FakeTransport::default();
        let empty_logger = logger(
            settings(),
            clone_store(&store),
            empty_transport.clone(),
            None,
        );
        export_usage_data(
            &empty_logger,
            None,
            None,
            Some(minutes_ago(120).and_utc()),
            Some(minutes_ago(60).and_utc()),
        )
        .await
        .expect("an empty window");
        assert!(empty_transport.posts().is_empty());
        let failing_transport = FakeTransport::failing();
        let failing_logger = logger(
            settings(),
            clone_store(&store),
            failing_transport.clone(),
            None,
        );
        let error = export_usage_data(&failing_logger, None, None, None, None)
            .await
            .expect_err("a refused post");
        assert!(matches!(error, CloudZeroExportError::Delivery(_)));
        assert_eq!(failing_transport.posts().len(), 1);
        close(fixture, &store).await;
    }
}
