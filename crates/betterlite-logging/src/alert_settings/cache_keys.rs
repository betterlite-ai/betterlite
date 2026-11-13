#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum SlackAlertingCacheKeys {
    FailedRequestsDailyMetrics,
    LatencyDailyMetrics,
    DailyMetricsReportSent,
    ModelDeprecationAlertSent,
}
