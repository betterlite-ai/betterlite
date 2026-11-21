use betterlite_logging::prometheus::budget_gauges::inputs::TeamMembersGaugeInput;
use betterlite_logging::prometheus::budget_gauges::small_gauges::set_team_members_metric;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::Input;
use super::output::samples_value;
use super::probe::ProbeLog;
use super::support::{SHAPE_ERROR_MESSAGE, TEAM_MEMBERS, field, helper_error, label_source};

pub(super) fn team_members_value(input: &Input) -> HelperResult {
    let Input::TeamMembers {
        team_id,
        team_alias,
        member_count,
        labels_by_metric,
    } = input
    else {
        return Err(helper_error("InvalidInput", SHAPE_ERROR_MESSAGE));
    };
    let log = ProbeLog::default();
    let gauge = field(TEAM_MEMBERS, labels_by_metric, &log)?;
    let settings = PrometheusLabelSettings::with_defaults();
    let call = TeamMembersGaugeInput {
        team_id: team_id.clone(),
        team_alias: team_alias.clone(),
        member_count: *member_count,
    };
    set_team_members_metric(&gauge, &call, label_source(&settings, false))
        .map_err(|error| helper_error("RuntimeError", &error.to_string()))?;
    Ok(samples_value(&log.drain()))
}
