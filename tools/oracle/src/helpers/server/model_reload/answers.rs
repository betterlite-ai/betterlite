use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_server::model_reload::cache::{ReconcileOutcome, ReloadFailure, clear_cache};
use betterlite_server::model_reload::model_info::model_info_as_mapping;
use betterlite_server::model_reload::verdict::{
    ReloadState, ServeObligation, WrittenModel, reload_serving_verdict,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

// ast-grep-ignore: no-long-names
use betterlite_server::model_reload::verdict::raise_if_reload_degraded_serving as raise_if_degraded;

use super::support::{Database, FakeRouter, Strategies};
use super::{
    Call, ClearCacheCall, Input, LiveInput, MappingCall, ObligationInput, OutcomeInput, RaiseCall,
    VerdictCall, WrittenInput,
};

pub(super) async fn answer(input: &Input) -> HelperResult {
    let environment = input
        .environment
        .get("LITELLM_ENVIRONMENT")
        .map(String::as_str);
    let mut answers = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        answers.push(answer_call(call, environment).await);
    }
    Ok(json!({ "calls": answers }))
}

async fn answer_call(call: &Call, environment: Option<&str>) -> Value {
    match call {
        Call::ModelInfoAsMapping(call) => mapping_answer(call),
        Call::ReloadServingVerdict(call) => verdict_answer(call, environment),
        Call::RaiseIfReloadDegradedServing(call) => raise_answer(call, environment),
        Call::ClearCache(call) => clear_cache_answer(call).await,
    }
}

fn mapping_answer(call: &MappingCall) -> Value {
    match model_info_as_mapping(&call.model_info) {
        Some(mapping) => json!(mapping),
        None => json!(null),
    }
}

fn verdict_answer(call: &VerdictCall, environment: Option<&str>) -> Value {
    let prepared = PreparedWrites::prepare(
        &call.before,
        &call.written,
        call.still_desired.as_deref(),
        &call.live,
    );
    let verdict = reload_serving_verdict(
        &prepared.state(environment),
        ServeObligation::from(call.obligation),
    );
    json!({ "missing": verdict.missing, "collateral": verdict.collateral })
}

fn raise_answer(call: &RaiseCall, environment: Option<&str>) -> Value {
    let prepared = PreparedWrites::prepare(
        &call.before,
        &call.written,
        call.still_desired.as_deref(),
        &call.live,
    );
    match raise_if_degraded(&prepared.state(environment), &call.action) {
        Ok(()) => json!({ "raised": false, "type": null, "param": null, "code": null }),
        Err(error) => json!({
            "raised": true,
            "type": error.error_type,
            "param": error.param,
            "code": error.code,
        }),
    }
}

async fn clear_cache_answer(call: &ClearCacheCall) -> Value {
    let mut router = call
        .router_present
        .then(|| FakeRouter::new(&call.models, &call.strategy_names, reload_result(call)));
    let database = Database::new();
    let repository = call.database_present.then(|| database.repository());
    let gate = Mutex::new(());
    let returned = clear_cache(&gate, router.as_mut(), repository.as_ref()).await;
    let (deleted, remaining, registries, reloads) = match &router {
        Some(fake) => {
            let published = fake.catalog.load_full();
            let remaining: Vec<String> = published.id_index.keys().cloned().collect();
            let registries = registry_answer(&fake.strategies);
            (fake.deleted.clone(), remaining, registries, fake.reloads)
        }
        None => (
            Vec::new(),
            Vec::new(),
            json!({"adaptive": [], "auto": [], "complexity": [], "quality": []}),
            0_usize,
        ),
    };
    json!({
        "deleted": deleted,
        "remaining": remaining,
        "registries": registries,
        "reloads": reloads,
        "outcome": outcome_answer(&returned),
    })
}

struct PreparedWrites {
    before: BTreeSet<String>,
    written: Vec<WrittenModel>,
    still_desired: Option<BTreeSet<String>>,
    captured: Option<BTreeSet<String>>,
    catalog: CatalogHandle,
}

impl PreparedWrites {
    fn prepare(
        before: &[String],
        written: &[WrittenInput],
        still_desired: Option<&[String]>,
        live: &LiveInput,
    ) -> Self {
        Self {
            before: id_set(before),
            written: written.iter().map(written_model).collect(),
            still_desired: still_desired.map(id_set),
            captured: live.captured.as_deref().map(id_set),
            catalog: live_catalog(live.router_ids.as_deref().unwrap_or_default()),
        }
    }

    fn state<'state>(&'state self, environment: Option<&'state str>) -> ReloadState<'state> {
        ReloadState {
            before: &self.before,
            written: &self.written,
            still_desired: self.still_desired.as_ref(),
            live_after: self.captured.as_ref(),
            catalog: Some(&self.catalog),
            environment,
        }
    }
}

impl From<ObligationInput> for ServeObligation {
    fn from(input: ObligationInput) -> Self {
        match input {
            ObligationInput::MustServe => Self::MustServe,
            ObligationInput::MustNotDegrade => Self::MustNotDegrade,
        }
    }
}

fn written_model(model: &WrittenInput) -> WrittenModel {
    WrittenModel {
        id: model.id.clone(),
        model_info: model.model_info.clone(),
    }
}

fn id_set(ids: &[String]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for id in ids {
        set.insert(id.clone());
    }
    set
}

fn live_catalog(ids: &[String]) -> CatalogHandle {
    let mut catalog = DeploymentCatalog::default();
    for id in ids {
        let mut model_info = JsonObject::new();
        model_info.insert("id".to_owned(), Json::Text(id.clone()));
        catalog.push(DeploymentSpec {
            model_name: id.clone(),
            litellm_params: JsonObject::new(),
            model_info,
            extra: JsonObject::new(),
        });
    }
    CatalogHandle::from_pointee(catalog)
}

fn reload_result(call: &ClearCacheCall) -> Result<ReconcileOutcome, ReloadFailure> {
    match &call.reload_error {
        Some(message) => Err(ReloadFailure {
            message: message.clone(),
        }),
        None => Ok(reconcile_outcome(&call.outcome)),
    }
}

fn reconcile_outcome(input: &OutcomeInput) -> ReconcileOutcome {
    ReconcileOutcome {
        still_desired: input.still_desired.as_deref().map(id_set),
        live_after: input.live_after.as_deref().map(id_set),
    }
}

fn outcome_answer(outcome: &ReconcileOutcome) -> Value {
    json!({
        "still_desired": &outcome.still_desired,
        "live_after": &outcome.live_after,
    })
}

fn registry_answer(strategies: &Strategies) -> Value {
    json!({
        "adaptive": strategies.adaptive,
        "auto": strategies.auto,
        "complexity": strategies.complexity,
        "quality": strategies.quality,
    })
}
