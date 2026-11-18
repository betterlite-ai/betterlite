use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::model_visibility::Settings;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_server::model_visibility::discoverable::{
    discoverable_rows, undiscoverable_model_names,
};
use betterlite_server::model_visibility::healthy::hidden_unhealthy_model_names;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::error::Failure;
use super::fakes::{HealthReport, captured, user};
use super::input::{Function, Input};
use super::snapshot::{Snapshot, spec};

// The answer the case asked for: the owned entry point, projected to the typed values both sides compare.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    match input.function {
        Function::UndiscoverableModelNames => undiscoverable(input),
        Function::DiscoverableRows => discoverable(input),
        Function::HiddenUnhealthyModelNames => healthy(input),
    }
}

fn undiscoverable(input: &Input) -> Result<Value, Failure> {
    let snapshot = Snapshot::build(input)?;
    let lookup = CatalogLookup {
        catalog: &snapshot.catalog,
        model_group_alias: &snapshot.aliases,
        routing_groups: &snapshot.groups,
        inferred_provider: &*snapshot.provider,
    };
    let router = input
        .arguments
        .with_router
        .unwrap_or(true)
        .then_some(&lookup);
    let hidden = undiscoverable_model_names(
        &input.arguments.model_names,
        router,
        &user(input.arguments.user_role.as_deref())?,
        input.arguments.team_id.as_deref(),
    )?;
    Ok(json!({ "hidden": hidden }))
}

fn discoverable(input: &Input) -> Result<Value, Failure> {
    let mut rows: Vec<DeploymentSpec> = Vec::with_capacity(input.model_list.len());
    for row in &input.model_list {
        rows.push(spec(&row.model_name, &row.litellm_params, &row.model_info)?);
    }
    let kept = discoverable_rows(&rows, &user(input.arguments.user_role.as_deref())?);
    let names: Vec<&str> = kept.iter().map(|row| row.model_name.as_str()).collect();
    Ok(json!({ "rows": names }))
}

fn healthy(input: &Input) -> Result<Value, Failure> {
    let arguments = &input.arguments;
    let runtime = Builder::new_current_thread().build()?;
    let reads = Arc::new(AtomicUsize::new(0));
    let report = HealthReport {
        names: arguments.health.unhealthy_names.iter().cloned().collect(),
        fails: arguments.health.fails,
        reads: Arc::clone(&reads),
    };
    let settings = Settings {
        model_list_healthy_only: arguments.model_list_healthy_only.unwrap_or(false),
    };
    let router = arguments.with_router.unwrap_or(true).then_some(&report);
    let (result, debug) = captured(&|| {
        runtime.block_on(hidden_unhealthy_model_names(
            arguments.healthy_only,
            settings,
            router,
        ))
    });
    let hidden = result.map_err(Failure::Health)?;
    Ok(json!({
        "hidden": hidden,
        "health_reads": reads.load(Ordering::Relaxed),
        "debug": debug,
    }))
}
