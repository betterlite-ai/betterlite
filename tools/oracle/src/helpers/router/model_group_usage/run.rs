use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::AccessGroups;
use betterlite_router::model_group_usage::access_filter::{
    AccessFilterRequest, KeyGrants, filter_by_access_groups,
};
use betterlite_router::model_group_usage::remaining::group_remaining_usage;
use betterlite_router::model_group_usage::response_headers::{
    ResponseHeadersRequest, set_response_headers,
};
use betterlite_router::model_group_usage::usage::{UsageSeries, group_counter_totals};
use betterlite_router::remaining_usage_headers::RemainingUsage;
use betterlite_router::router_core::headers::Response;
use serde::Serialize;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::cache::{FixedMetadata, ScriptedCache, moment};
use super::error::Failure;
use super::input::{Function, Input, PrincipalInput};
use super::snapshot::{Snapshot, group_info};

#[derive(Debug, Serialize)]
struct UsageOutput {
    tpm: Option<i64>,
    rpm: Option<i64>,
    reads: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct IoOutput {
    itpm: Option<i64>,
    otpm: Option<i64>,
    reads: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct RemainingOutput {
    headers: RemainingUsage,
    reads: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ResponseOutput {
    response: Json,
    reads: Vec<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct FilterOutput {
    ids: Vec<String>,
}

// What every answer needs once: the case's model list, scripted cache, clock reading and runtime.
struct CaseState {
    snapshot: Snapshot,
    cache: ScriptedCache,
    time: SystemTime,
    runtime: Runtime,
}

impl CaseState {
    // One ordered batch of the group's counters for the series, on the case's clock and cache.
    fn read_totals(
        &self,
        group: &str,
        series: UsageSeries,
    ) -> Result<(Option<i64>, Option<i64>), Failure> {
        let rows = self.snapshot.rows(group);
        self.runtime
            .block_on(group_counter_totals(
                series,
                Some(&rows),
                &self.cache,
                self.time,
            ))
            .map_err(Failure::from)
    }

    fn remaining_headers(&self, input: &Input, group: &str) -> Result<RemainingUsage, Failure> {
        let rows = self.snapshot.rows(group);
        self.runtime
            .block_on(group_remaining_usage(
                group,
                Some(&rows),
                &fixed_metadata(input),
                &self.cache,
                self.time,
            ))
            .map_err(Failure::from)
    }

    fn response_headers(
        &self,
        input: &Input,
        rows: Option<&[DeploymentSpec]>,
    ) -> Result<Response, Failure> {
        let response = input
            .response
            .as_ref()
            .map_or(Response::Absent, |body| Response::Object(body.clone()));
        self.runtime
            .block_on(set_response_headers(ResponseHeadersRequest {
                response,
                model_group: input.model_group.as_deref(),
                request_kwargs: input.request_kwargs.as_ref(),
                model_list: rows,
                metadata: &fixed_metadata(input),
                cache: &self.cache,
                now: self.time,
            }))
            .map_err(Failure::from)
    }
}

// Router(model_list=...) once, then the one owned method the case names, answered on the case's clock and cache.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let case = CaseState {
        snapshot: Snapshot::build(&input.model_list)?,
        cache: ScriptedCache::new(&input.cache),
        time: moment(input.now)?,
        runtime: Builder::new_current_thread().build()?,
    };
    match input.function {
        Function::Usage => usage_answer(input, &case),
        Function::IoUsage => io_usage_answer(input, &case),
        Function::Remaining => remaining_answer(input, &case),
        Function::Response => response_answer(input, &case),
        Function::Filter => filter_answer(input, &case.snapshot),
    }
}

// Router.get_model_group_usage: the group's batched tpm and rpm totals plus the reads they took.
fn usage_answer(input: &Input, case: &CaseState) -> Result<Value, Failure> {
    let group = group_of(input, "usage")?;
    let (tpm, rpm) = case.read_totals(group, UsageSeries::Standard)?;
    let reads = case.cache.reads();
    Ok(serde_json::to_value(UsageOutput { tpm, rpm, reads })?)
}

// Router.get_model_group_io_token_usage: the same batch over the itpm and otpm keys.
fn io_usage_answer(input: &Input, case: &CaseState) -> Result<Value, Failure> {
    let group = group_of(input, "io_usage")?;
    let (itpm, otpm) = case.read_totals(group, UsageSeries::IoToken)?;
    let reads = case.cache.reads();
    Ok(serde_json::to_value(IoOutput { itpm, otpm, reads })?)
}

// Router.get_remaining_model_group_usage: the io and ordinary headers the cached limits allow.
fn remaining_answer(input: &Input, case: &CaseState) -> Result<Value, Failure> {
    let group = group_of(input, "remaining")?;
    let headers = case.remaining_headers(input, group)?;
    let reads = case.cache.reads();
    Ok(serde_json::to_value(RemainingOutput { headers, reads })?)
}

// Router.set_response_headers over the case's parsed response object.
fn response_answer(input: &Input, case: &CaseState) -> Result<Value, Failure> {
    let rows = input
        .model_group
        .as_ref()
        .map(|group| case.snapshot.rows(group));
    let returned = case.response_headers(input, rows.as_deref())?;
    let response = returned_body(returned)?;
    let reads = case.cache.reads();
    Ok(serde_json::to_value(ResponseOutput { response, reads })?)
}

// Router._filter_deployments_by_model_access_groups: the ids of the candidates the grants leave.
fn filter_answer(input: &Input, snapshot: &Snapshot) -> Result<Value, Failure> {
    let ids = filtered_candidates(input, snapshot)?
        .iter()
        .map(|deployment| deployment.id().unwrap_or_default().to_owned())
        .collect();
    Ok(serde_json::to_value(FilterOutput { ids })?)
}

fn filtered_candidates(input: &Input, snapshot: &Snapshot) -> Result<Vec<DeploymentSpec>, Failure> {
    let group = group_of(input, "filter")?;
    let candidates = candidate_rows(input, snapshot, group)?;
    let metadata = input.metadata.as_ref().map(grants_of);
    let litellm_metadata = input.litellm_metadata.as_ref().map(grants_of);
    let no_inferred = |_model: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &snapshot.catalog,
        model_group_alias: &snapshot.aliases,
        routing_groups: &snapshot.groups,
        inferred_provider: &no_inferred,
    };
    let request = AccessFilterRequest {
        model: group,
        deployments: candidates.as_slice(),
        metadata: metadata.as_ref(),
        litellm_metadata: litellm_metadata.as_ref(),
        team_id: input.team_id.as_deref(),
    };
    Ok(filter_by_access_groups(
        &request,
        &AccessGroups::default(),
        &lookup,
    )?)
}

// The request's held deployments: the group's rows, or the case's own indices into them.
fn candidate_rows(
    input: &Input,
    snapshot: &Snapshot,
    group: &str,
) -> Result<Vec<DeploymentSpec>, Failure> {
    let rows = snapshot.rows(group);
    let Some(indices) = &input.candidates else {
        return Ok(rows);
    };
    indices
        .iter()
        .map(|index| {
            rows.get(*index)
                .cloned()
                .ok_or(Failure::Candidate { index: *index })
        })
        .collect()
}

fn grants_of(principal: &PrincipalInput) -> KeyGrants {
    KeyGrants {
        models: principal.models.clone(),
        team_models: principal.team_models.clone(),
    }
}

fn fixed_metadata(input: &Input) -> FixedMetadata {
    FixedMetadata {
        info: group_info(input.group_info.as_ref()),
    }
}

// The group a usage function needs, or the missing-group failure naming that function.
fn group_of<'input>(input: &'input Input, function: &'static str) -> Result<&'input str, Failure> {
    let Some(group) = input.model_group.as_deref() else {
        return Err(Failure::MissingGroup { function });
    };
    Ok(group)
}

fn returned_body(response: Response) -> Result<Json, Failure> {
    match response {
        Response::Absent => Ok(Json::Null),
        Response::Object(body) => Ok(Json::Object(body)),
        Response::Attributed(_) | Response::Stream | Response::Opaque => {
            Err(Failure::UnsupportedResponse)
        }
    }
}
