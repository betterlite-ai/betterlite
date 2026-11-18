use std::collections::BTreeSet;

use betterlite_oracle_helper_api::helper::HelperResult;
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
    let Some(fake) = router.as_ref() else {
        return json!({
            "deleted": [],
            "remaining": [],
            "registries": {"adaptive": [], "auto": [], "complexity": [], "quality": []},
            "reloads": 0_usize,
            "outcome": outcome_answer(&returned),
        });
    };
    json!({
        "deleted": fake.deleted,
        "remaining": fake.remaining(),
        "registries": registry_answer(&fake.strategies),
        "reloads": fake.reloads,
        "outcome": outcome_answer(&returned),
    })
}

struct PreparedWrites {
    before: BTreeSet<String>,
    written: Vec<WrittenModel>,
    still_desired: Option<BTreeSet<String>>,
    live_after: Option<BTreeSet<String>>,
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
            live_after: live
                .captured
                .as_deref()
                .or(live.router_ids.as_deref())
                .map(id_set),
        }
    }

    fn state<'state>(&'state self, environment: Option<&'state str>) -> ReloadState<'state> {
        ReloadState {
            before: &self.before,
            written: &self.written,
            still_desired: self.still_desired.as_ref(),
            live_after: self.live_after.as_ref(),
            catalog: None,
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

fn reload_result(call: &ClearCacheCall) -> Result<ReconcileOutcome, ReloadFailure> {
    let Some(message) = &call.reload_error else {
        return Ok(reconcile_outcome(&call.outcome));
    };
    Err(ReloadFailure {
        message: message.clone(),
    })
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
