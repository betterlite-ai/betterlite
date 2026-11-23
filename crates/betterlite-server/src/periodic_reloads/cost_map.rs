use betterlite_db::reload_schedule::pod_reload_decision::pod_reload_is_due;
use betterlite_models::reload::ReloadOutcome;

use crate::periodic_reloads::config_rows::ReloadConfigRows;
use crate::periodic_reloads::sources::ReloadSources;
use crate::periodic_reloads::state::PeriodicReloads;

const MODEL_COST_MAP_DESCRIPTION: &str = "Model cost map";

pub(crate) async fn reload<Rows: ReloadConfigRows>(
    db: &Rows,
    reloads: &PeriodicReloads,
    sources: &ReloadSources,
) -> Result<(), Rows::Error> {
    let Some(schedule) = db.read_schedule().await? else {
        return Ok(());
    };
    let now = sources.clock.utc_now();
    let due = pod_reload_is_due(
        &schedule,
        reloads.cost_map_applied_revision(),
        reloads.cost_map_loaded_at(),
        now,
        MODEL_COST_MAP_DESCRIPTION,
    );
    if !due {
        return Ok(());
    }
    match sources.cost_map.refetch_cost_map().await {
        ReloadOutcome::Unavailable(unavailable) => {
            let reason = unavailable.reason;
            tracing::warn!(
                "Model cost map reload failed ({reason}); keeping current pricing data. The revision stays unapplied so this pod retries on its next poll"
            );
            Ok(())
        }
        ReloadOutcome::Reloaded(snapshot) => {
            let models_count = reloads.swap_in_model_cost_map(&snapshot, now);
            db.record_run(now).await?;
            // the revision moves last, so a failed record leaves the next poll to retry the same run
            reloads.set_cost_map_applied_revision(schedule.reload_revision);
            tracing::info!("Model cost map reloaded successfully. Models count: {models_count}");
            Ok(())
        }
    }
}
