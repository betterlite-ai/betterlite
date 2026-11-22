use std::collections::BTreeSet;

use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::errors::{GroupDataError, HttpFailure};
use crate::model_reload::verdict::{
    ReloadState, ServeObligation, WrittenModel, reload_serving_verdict,
};

// _raise_http_if_reload_degraded_serving: the metadata-only obligation in this file's HTTP error convention; an already-dead written row and an unwanted drop are not reported.
pub fn raise_if_degraded_serving(
    before: &BTreeSet<String>,
    written_models: &[(String, JsonObject)],
    access_group: &str,
    still_desired: Option<&BTreeSet<String>>,
    live_after: Option<&BTreeSet<String>>,
) -> Result<(), GroupDataError> {
    let written: Vec<WrittenModel> = written_models
        .iter()
        .map(|(id, model_info)| WrittenModel {
            id: id.clone(),
            model_info: Json::Object(model_info.clone()),
        })
        .collect();
    let state = ReloadState {
        before,
        written: &written,
        still_desired,
        live_after,
        catalog: None,
        environment: None,
    };
    let verdict = reload_serving_verdict(&state, ServeObligation::MustNotDegrade);
    let mut gone: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for id in verdict.missing.into_iter().chain(verdict.collateral) {
        if seen.insert(id.clone()) {
            gone.push(id);
        }
    }
    if gone.is_empty() {
        return Ok(());
    }
    Err(GroupDataError::Http(HttpFailure {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: degraded_message(access_group, &gone),
    }))
}

fn degraded_message(access_group: &str, gone: &[String]) -> String {
    let listed: Vec<String> = gone.iter().map(|id| format!("'{id}'")).collect();
    let joined = listed.join(", ");
    format!(
        "Access group '{access_group}' was saved to the database, but model id(s) [{joined}] that this pod was serving are no longer live after the reload it triggered. Other pods reload on their own interval. Check server logs for 'Error upserting deployment' for the cause."
    )
}
