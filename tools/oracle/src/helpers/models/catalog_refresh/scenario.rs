use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::catalog_source::Raw;
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use betterlite_models::reload::{
    CatalogService, FetchLimits, NoRuntimeRegistrations, ReloadOutcome,
};
use betterlite_models::source::{
    CatalogClock, CatalogProvenance, CatalogSnapshot, CatalogSource, CatalogStore, SourceKind,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::input::{Input, Operation};
use super::invalid;
use super::scripting::{ScriptedClient, ScriptedJitter, ScriptedSleeper};
use crate::helpers::{HelperError, HelperResult};

const MODEL_LIMIT: usize = 50;

type Service = CatalogService<fn() -> Result<Vec<u8>, CatalogError>>;

struct Facts<'facts> {
    kind: &'facts str,
    reason: Option<&'facts str>,
    immediate: &'facts CatalogSnapshot,
    final_snapshot: &'facts CatalogSnapshot,
    client: &'facts ScriptedClient,
    sleeper: &'facts ScriptedSleeper,
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input.operation {
        Operation::Startup => startup(input),
        Operation::Reload => reload(input),
    }
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(error.to_string()))
}

fn service(input: &Input) -> Result<Arc<Service>, Box<HelperError>> {
    let settings = Raw::default()
        .convert(&input.environment)
        .map_err(|error| invalid(error.to_string()))?;
    let clock: CatalogClock = Arc::new(DateTime::<Utc>::default);
    Ok(Arc::new(CatalogService {
        source: CatalogSource::from_settings(settings.local, &settings.url),
        bundled: Mutex::new(BundledCatalog {
            asset: BundledCatalog::default().asset,
            cached: None,
        }),
        clock,
        registrations: Arc::new(NoRuntimeRegistrations),
        current: CatalogStore::default(),
    }))
}

const fn limits(input: &Input) -> FetchLimits {
    FetchLimits {
        timeout: Duration::from_secs(input.timeout),
        max_attempts: input.max_attempts,
    }
}

fn startup(input: &Input) -> HelperResult {
    let service = service(input)?;
    let client = Arc::new(ScriptedClient::new(&input.replies));
    let sleeper = Arc::new(ScriptedSleeper::default());
    let runtime = runtime()?;
    let outcome = runtime.block_on(service.startup(
        limits(input),
        Arc::clone(&client),
        Arc::clone(&sleeper),
        ScriptedJitter::new(&input.jitter)?,
    ));
    let immediate = Arc::clone(&outcome.snapshot);
    if let Some(retry) = outcome.retry {
        let _adoption = runtime.block_on(retry.run(Arc::clone(&service)));
    }
    let final_snapshot = service.current.load();
    Ok(facts(
        input,
        &Facts {
            kind: "startup",
            reason: None,
            immediate: &immediate,
            final_snapshot: &final_snapshot,
            client: &client,
            sleeper: &sleeper,
        },
    ))
}

fn reload(input: &Input) -> HelperResult {
    let service = service(input)?;
    let client = Arc::new(ScriptedClient::new(&input.replies));
    let sleeper = Arc::new(ScriptedSleeper::default());
    let runtime = runtime()?;
    let startup = runtime.block_on(service.startup(
        limits(input),
        Arc::clone(&client),
        Arc::clone(&sleeper),
        ScriptedJitter::new(&input.jitter)?,
    ));
    let immediate = Arc::clone(&startup.snapshot);
    if let Some(retry) = startup.retry {
        let _adoption = runtime.block_on(retry.run(Arc::clone(&service)));
    }
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let outcome = runtime.block_on(service.reload(
        limits(input),
        client.as_ref(),
        sleeper.as_ref(),
        &mut jitter,
    ));
    let final_snapshot = service.current.load();
    let (kind, reason) = match &outcome {
        ReloadOutcome::Reloaded(_) => ("reloaded", None),
        ReloadOutcome::Unavailable(unavailable) => {
            ("unavailable", Some(unavailable.reason.as_str()))
        }
    };
    Ok(facts(
        input,
        &Facts {
            kind,
            reason,
            immediate: &immediate,
            final_snapshot: &final_snapshot,
            client: &client,
            sleeper: &sleeper,
        },
    ))
}

fn facts(input: &Input, facts: &Facts<'_>) -> Value {
    let catalog = &facts.final_snapshot.catalog;
    let sleeps = facts
        .sleeper
        .recorded()
        .iter()
        .map(|seconds| seconds.normalize().to_string())
        .collect::<Vec<_>>();
    let probes = input
        .probe_models
        .iter()
        .map(|name| {
            let provider = catalog
                .metadata(name)
                .map(|projection| projection.provider.to_owned());
            (name.clone(), provider)
        })
        .collect::<BTreeMap<_, _>>();
    let generalization = input
        .probe_generalization
        .as_deref()
        .and_then(|probe| catalog.generalizations.match_routing(probe))
        .map(str::to_owned);
    json!({
        "kind": facts.kind,
        "reason_kind": facts.reason.map(classify_reason),
        "before": provenance_facts(&facts.immediate.provenance),
        "after": provenance_facts(&facts.final_snapshot.provenance),
        "catalog": {"count": catalog.adopted_size, "models": model_names(catalog)},
        "probes": probes,
        "generalization": generalization,
        "requests": facts.client.requests.load(Ordering::SeqCst),
        "sleeps": sleeps,
    })
}

fn provenance_facts(provenance: &CatalogProvenance) -> Value {
    json!({
        "source": match provenance.kind {
            SourceKind::Local => "local",
            SourceKind::Remote => "remote",
        },
        "url": &provenance.url,
        "is_env_forced": provenance.is_env_forced,
        "fallback": provenance.fallback_reason.as_deref().map(classify_reason),
        "loaded_at_set": provenance.loaded_at.is_some(),
        "revision": &provenance.source_revision,
        "etag": &provenance.etag,
    })
}

fn model_names(catalog: &ModelCatalog) -> Value {
    if catalog.adopted_size > MODEL_LIMIT {
        return Value::Null;
    }
    let mut names = catalog
        .lookup
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    names.sort_unstable();
    json!(names)
}

fn classify_reason(reason: &str) -> &'static str {
    if reason.starts_with("HTTP ") {
        "http"
    } else if reason.starts_with("invalid JSON") {
        "invalid_json"
    } else if reason.starts_with("Remote fetch failed:") {
        "fetch"
    } else if reason.contains("integrity validation") {
        "integrity"
    } else if reason.contains("failed to load") {
        "load"
    } else {
        "other"
    }
}
