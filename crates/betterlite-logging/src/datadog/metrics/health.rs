use crate::exporter_options::health::IntegrationHealthStatus;

use super::datadog_metrics::DatadogMetricsLogger;
use super::delivery::upload;
use super::series::{
    DatadogMetricPoint, DatadogMetricSeries, GAUGE_TYPE, HEALTH_CHECK_METRIC, HEALTH_CHECK_TAG,
};

pub async fn async_health_check(logger: &DatadogMetricsLogger) -> IntegrationHealthStatus {
    let Ok(timestamp) = i64::try_from(logger.clock.now_unix().as_secs()) else {
        return IntegrationHealthStatus::unhealthy(
            "the health check timestamp does not fit a unix timestamp".to_owned(),
        );
    };
    let series = DatadogMetricSeries {
        metric: HEALTH_CHECK_METRIC,
        kind: GAUGE_TYPE,
        points: vec![DatadogMetricPoint {
            timestamp,
            value: 1.0_f64,
        }],
        tags: vec![HEALTH_CHECK_TAG.to_owned()],
        interval: None,
    };
    match upload(logger, &[series]).await {
        Ok(()) => IntegrationHealthStatus::healthy(),
        Err(error) => IntegrationHealthStatus::unhealthy(error.to_string()),
    }
}
