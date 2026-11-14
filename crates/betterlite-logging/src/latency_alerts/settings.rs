use betterlite_config::sections::alert_settings::types::AlertType;

#[derive(Clone, Debug, PartialEq)]
pub struct RequestAlertSettings {
    pub alerting_configured: bool,
    pub alert_types: Vec<AlertType>,
    pub alerting_threshold_seconds: f64,
}

pub trait RequestAlertSettingsSource: Send + Sync {
    fn current(&self) -> RequestAlertSettings;
}
