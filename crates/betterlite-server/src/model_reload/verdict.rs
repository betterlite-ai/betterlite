use std::collections::{BTreeMap, BTreeSet};

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::CatalogHandle;

use super::live_ids::live_model_ids_snapshot;
use super::model_info::expects_liveness_on_this_pod;

// The obligation the write puts on each id it saved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServeObligation {
    MustServe,
    MustNotDegrade,
}

// One row the write saved: its model id and its model_info column as it was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrittenModel {
    pub id: String,
    pub model_info: Json,
}

// Everything reload_serving_verdict compares: the router before the write, what the write saved, the desired set, and where the live ids come from.
pub struct ReloadState<'state> {
    pub before: &'state BTreeSet<String>,
    pub written: &'state [WrittenModel],
    pub still_desired: Option<&'state BTreeSet<String>>,
    pub live_after: Option<&'state BTreeSet<String>>,
    pub catalog: Option<&'state CatalogHandle>,
    pub environment: Option<&'state str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelReloadVerdict {
    pub missing: Vec<String>,
    pub collateral: Vec<String>,
}

// reload_serving_verdict: judge a write-triggered reload by the router's serving state instead of the reload's own report.
#[must_use]
pub fn reload_serving_verdict(
    state: &ReloadState<'_>,
    obligation: ServeObligation,
) -> ModelReloadVerdict {
    let now = match state.live_after {
        Some(ids) => ids.clone(),
        None => live_model_ids_snapshot(state.catalog),
    };
    let mut missing = Vec::new();
    for written in state.written {
        if violates(state, obligation, written, &now) {
            missing.push(written.id.clone());
        }
    }
    let written_ids: BTreeSet<&str> = state
        .written
        .iter()
        .map(|written| written.id.as_str())
        .collect();
    let mut collateral = Vec::new();
    for id in state.before.difference(&now) {
        if written_ids.contains(id.as_str()) || !still_desired_includes(state.still_desired, id) {
            continue;
        }
        collateral.push(id.clone());
    }
    ModelReloadVerdict {
        missing,
        collateral,
    }
}

// ast-grep-ignore: no-long-names - raise_if_reload_degraded_serving: the caller-visible 500 owed when the model the write saved is not being served after its reload.
pub fn raise_if_reload_degraded_serving(
    state: &ReloadState<'_>,
    action: &str,
) -> Result<(), Box<ProxyException>> {
    let verdict = reload_serving_verdict(state, ServeObligation::MustServe);
    if verdict.missing.is_empty() && verdict.collateral.is_empty() {
        return Ok(());
    }
    Err(Box::new(ProxyException::new(ProxyExceptionInit {
        message: degraded_message(&verdict, action),
        error_type: ProxyErrorTypes::InternalServerError.to_string(),
        param: None,
        code: ExceptionCode::Status(500),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })))
}

fn violates(
    state: &ReloadState<'_>,
    obligation: ServeObligation,
    written: &WrittenModel,
    now: &BTreeSet<String>,
) -> bool {
    match obligation {
        ServeObligation::MustServe => {
            !now.contains(&written.id)
                && expects_liveness_on_this_pod(&written.model_info, state.environment)
        }
        ServeObligation::MustNotDegrade => {
            state.before.contains(&written.id) && !now.contains(&written.id)
        }
    }
}

fn still_desired_includes(still_desired: Option<&BTreeSet<String>>, id: &str) -> bool {
    match still_desired {
        Some(desired) => desired.contains(id),
        None => true,
    }
}

fn degraded_message(verdict: &ModelReloadVerdict, action: &str) -> String {
    let missing_clause = if verdict.missing.is_empty() {
        "the reload it triggered degraded this pod's serving state.".to_owned()
    } else {
        format!(
            "the model id(s) {} are not live in this pod's router after the reload and are not being served by this pod.",
            verdict.missing.join(", ")
        )
    };
    let collateral_clause = if verdict.collateral.is_empty() {
        String::new()
    } else {
        format!(
            " Previously served model id(s) {} are also no longer being served by this pod.",
            verdict.collateral.join(", ")
        )
    };
    format!(
        "Model {action} was saved to the database, but {missing_clause}{collateral_clause} Other pods reload on their own interval. Check server logs for 'Error upserting deployment' or 'Error creating deployment' for the cause."
    )
}
