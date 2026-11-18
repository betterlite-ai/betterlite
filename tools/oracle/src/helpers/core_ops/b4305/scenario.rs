use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_cache::handler::retrieve::CacheSource;
use betterlite_core::caching::lookup::async_lookup::{AsyncLookup, async_get_cache};
use betterlite_core::caching::lookup::services::LookupServices;
use betterlite_core::caching::lookup::sync_lookup::sync_get_cache;
use betterlite_logging::worker::LoggingWorker;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use chrono::DateTime;
use serde_json::Value;

use super::fixtures::{
    Conversions, FIXED_TIME, FixedClock, Input, Operation, Resolution, Scenario, Tokens,
    cache_fixture, caching_handler, hit_logging, logged_from, lookup_report, lookup_request,
    read_support, worker_settings,
};
pub(crate) async fn scenarios(input: &Input) -> Result<Vec<Value>, Box<HelperError>> {
    let mut reports = Vec::with_capacity(input.scenarios.len());
    for scenario in &input.scenarios {
        reports.push(scenario_report(scenario).await?);
    }
    Ok(reports)
}
async fn scenario_report(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let worker = LoggingWorker::start(worker_settings()).or_config_error()?;
    let resolution = Resolution::new()?;
    let context = resolution.context();
    let tokens = Tokens;
    let cache = scenario
        .cache_configured
        .then(|| cache_fixture(scenario))
        .transpose()?;
    let source = cache.as_ref().map(|cache| CacheSource {
        cache,
        async_support: read_support(scenario.operation),
    });
    let mut handler = caching_handler(scenario)?;
    let request = lookup_request(scenario)?;
    let services = LookupServices {
        cache: source.as_ref(),
        cache_type: scenario.cache_type,
        supported_call_types: scenario.supported_call_types.as_deref(),
        resolution: &context,
        worker: &worker,
        tokens: &tokens,
        perf: Arc::new(|| Duration::ZERO),
        wall: Arc::new(FixedClock {
            now: DateTime::parse_from_rfc3339(FIXED_TIME).or_value_error()?,
        }),
    };
    match scenario.operation {
        Operation::Async => {
            let outcome = async_get_cache(
                &mut handler,
                request,
                hit_logging(scenario, &snapshots)?,
                &services,
                &Conversions,
            )
            .await
            .or_value_error()?;
            worker.flush().await;
            let AsyncLookup { response, logging } = outcome;
            let view = logged_from(&snapshots, logging.as_ref().map(|logging| &logging.state))?;
            lookup_report(response.as_ref(), response.is_some(), &view)
        }
        Operation::Sync => {
            let mut logging = hit_logging(scenario, &snapshots)?;
            let response =
                sync_get_cache(&mut handler, request, &mut logging, &services, &Conversions)
                    .await
                    .or_value_error()?;
            worker.flush().await;
            let view = logged_from(&snapshots, Some(&logging.state))?;
            lookup_report(Some(&response), true, &view)
        }
    }
}
