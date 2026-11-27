use betterlite_db::team_rows::member::Member;
use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus::budget_gauges::inputs::TeamMembersGaugeInput;
use betterlite_logging::prometheus::budget_gauges::small_gauges::set_team_members_metric;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_prometheus::collector_factory::collector::Collector;

mod tests;

// LiteLLM looks the PrometheusLogger up in a global and does nothing without it; here the caller hands over the gauge it registered
pub fn emit_team_members_metric<Gauge>(
    gauge: Option<&GaugeField<Gauge>>,
    source: LabelSource<'_>,
    team_id: &str,
    team_alias: Option<&str>,
    members: &[Member],
) where
    Gauge: Collector,
{
    let Some(field) = gauge else {
        return;
    };
    let input = TeamMembersGaugeInput {
        team_id: team_id.to_owned(),
        team_alias: team_alias.map(str::to_owned),
        member_count: members.len(),
    };
    if let Err(error) = set_team_members_metric(field, &input, source) {
        tracing::debug!("Prometheus: failed to emit team members metric: {error}");
    }
}
