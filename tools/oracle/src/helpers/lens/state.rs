use betterlite_lens::models::decode::decode;
use betterlite_lens::models::finding::FindingDraft;
use betterlite_lens::models::job::{Lens, Worker};
use betterlite_lens::models::messages::RunRequest;
use betterlite_lens::models::settings::{LensSettings, Scope};
use betterlite_lens::state::finding_identity::merge_finding;
use betterlite_lens::state::snapshot_finding::snapshot_finding;
use betterlite_lens::state::transitions::{
    TransitionError, can_access, claim_job, queue_job, renew_budget,
};
use chrono::{DateTime, Utc};

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindingCall {
    lens: Lens,
    draft: FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CanAccess {
        viewer: Scope,
        target: Scope,
    },
    QueueJob {
        lens: Lens,
        now: DateTime<Utc>,
        job_id: String,
        request: RunRequest,
    },
    ClaimJob {
        lens: Lens,
        worker: Worker,
        now: DateTime<Utc>,
    },
    RenewBudget {
        lens: Lens,
        now: DateTime<Utc>,
    },
    MergeFinding(FindingCall),
    SnapshotFinding(FindingCall),
    DecodeSettings {
        json: serde_json::Value,
    },
    AnalysisChecks {
        settings: LensSettings,
    },
}

fn failure(class: &str, message: String) -> HelperResult {
    let error = HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Err(Box::new(error))
}

fn decode_settings(json: &serde_json::Value) -> HelperResult {
    match serde_json::to_vec(json) {
        Err(error) => failure("TypeError", error.to_string()),
        Ok(bytes) => decode::<LensSettings>(&bytes).map_or_else(
            |error| failure("ValidationError", error.to_string()),
            |settings| {
                serde_json::to_value(settings)
                    .or_else(|error| failure("TypeError", error.to_string()))
            },
        ),
    }
}

fn run(input: &Input) -> HelperResult {
    let overflow = |error: TransitionError| failure("OverflowError", error.to_string());
    match input {
        Input::CanAccess { viewer, target } => serde_json::to_value(can_access(viewer, target))
            .or_else(|error| failure("TypeError", error.to_string())),
        Input::QueueJob {
            lens,
            now,
            job_id,
            request,
        } => queue_job(lens, *now, job_id.clone(), request).map_or_else(overflow, |queued| {
            serde_json::to_value(queued).or_else(|error| failure("TypeError", error.to_string()))
        }),
        Input::ClaimJob { lens, worker, now } => {
            claim_job(lens, worker, *now).map_or_else(overflow, |claimed| {
                serde_json::to_value(claimed)
                    .or_else(|error| failure("TypeError", error.to_string()))
            })
        }
        Input::RenewBudget { lens, now } => serde_json::to_value(renew_budget(lens, *now))
            .or_else(|error| failure("TypeError", error.to_string())),
        Input::MergeFinding(call) => serde_json::to_value(merge_finding(
            &call.lens,
            &call.draft,
            call.revision,
            call.now,
        ))
        .or_else(|error| failure("TypeError", error.to_string())),
        Input::SnapshotFinding(call) => serde_json::to_value(snapshot_finding(
            &call.lens,
            &call.draft,
            call.revision,
            call.now,
        ))
        .or_else(|error| failure("TypeError", error.to_string())),
        Input::DecodeSettings { json } => decode_settings(json),
        Input::AnalysisChecks { settings } => serde_json::to_value(settings.analysis_checks())
            .or_else(|error| failure("TypeError", error.to_string())),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run(input)
    }
}
