use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::payload::deps::TagConfig;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;

// the settings one logger writes with: the label schema B3875 registered the collectors from, the end-user switches, the tag rules and the bounded-series limits
#[derive(Clone, Debug)]
pub struct PrometheusHookSettings {
    pub labels: PrometheusLabelSettings,
    pub end_user: EndUserCostTrackingSettings,
    pub tags: TagConfig,
    pub limits: SeriesLimits,
}

impl Default for PrometheusHookSettings {
    // the schema's own defaults, since an empty caller identity names no label
    fn default() -> Self {
        Self {
            labels: PrometheusLabelSettings::with_defaults(),
            end_user: EndUserCostTrackingSettings::default(),
            tags: TagConfig::default(),
            limits: SeriesLimits::default(),
        }
    }
}
