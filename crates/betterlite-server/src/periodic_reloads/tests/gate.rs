use betterlite_config::sections::settings::SettingsStore;
use rstest::rstest;

use super::support::{
    Harness, everything_settings, general_settings, moment, reloaded, run_cost_map_check, schedule,
};

#[tokio::test]
async fn a_gate_that_names_other_objects_skips_the_cost_map_check() {
    let harness = Harness::new(moment(1));
    harness.rows.set_schedule(schedule(1, None, None));
    harness.cost_map.queue(reloaded());
    let settings = general_settings(&["models", "mcp"]);
    run_cost_map_check(&harness, &settings).await;
    assert_eq!(harness.cost_map.calls(), 0);
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 0);
}

#[rstest]
#[case::the_object_is_named(general_settings(&["model_cost_map"]))]
#[case::no_gate_is_configured(everything_settings())]
#[tokio::test]
async fn a_gate_that_allows_the_object_runs_the_cost_map_check(#[case] settings: SettingsStore) {
    let harness = Harness::new(moment(1));
    harness.rows.set_schedule(schedule(1, None, None));
    harness.cost_map.queue(reloaded());
    run_cost_map_check(&harness, &settings).await;
    assert_eq!(harness.cost_map.calls(), 1);
    assert_eq!(harness.checks.reloads.cost_map_applied_revision(), 1);
}
