use betterlite_config::sections::alert_settings::types::SlackAlertingArgs;

use super::message::OutageAlertType;

pub(crate) struct OutageAlertState<'alerts> {
    pub(crate) alerts: &'alerts [i64],
    pub(crate) minor_alert_sent: bool,
    pub(crate) major_alert_sent: bool,
}

// Only a timeout or a 500 and above counts; LiteLLM's docstring lists 401 and 429 but its filter drops them.
#[must_use]
pub(crate) const fn outage_status(status_code: Option<i64>) -> Option<i64> {
    match status_code {
        Some(code) if code == 408 || code >= 500 => Some(code),
        Some(_) | None => None,
    }
}

pub(crate) fn append_status(alerts: &mut Vec<i64>, status_code: i64, max_size: u32) {
    if alerts.len() < usize::try_from(max_size).unwrap_or(usize::MAX) {
        alerts.push(status_code);
    }
}

// The minor check is an if/elif with the major one, so one call sends at most one alert and major waits for minor to be off.
#[must_use]
pub(crate) fn alert_type_for(
    state: &OutageAlertState<'_>,
    args: &SlackAlertingArgs,
    multiple_deployments: bool,
) -> Option<OutageAlertType> {
    let alert_count = state.alerts.len();
    let minor_threshold = usize::try_from(args.minor_outage_alert_threshold).unwrap_or(usize::MAX);
    if multiple_deployments && !state.minor_alert_sent && alert_count >= minor_threshold {
        return Some(OutageAlertType::Minor);
    }
    let major_threshold = usize::try_from(args.major_outage_alert_threshold).unwrap_or(usize::MAX);
    if multiple_deployments && !state.major_alert_sent && alert_count >= major_threshold {
        return Some(OutageAlertType::Major);
    }
    None
}
