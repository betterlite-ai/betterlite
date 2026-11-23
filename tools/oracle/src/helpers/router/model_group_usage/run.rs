use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::model_group_lookups::access_groups::AccessGroups;
use betterlite_router::model_group_usage::access_filter::{
    AccessFilterRequest, KeyGrants, filter_deployments_by_model_access_groups,
};
use betterlite_router::model_group_usage::remaining::get_remaining_model_group_usage;
use betterlite_router::model_group_usage::response_headers::{
    ResponseHeadersRequest, set_response_headers,
};
use betterlite_router::model_group_usage::usage::{
    get_model_group_io_token_usage, get_model_group_usage,
};
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

// Router(model_list=...) once, then the one owned method the case names, answered on the case's clock.
pub(crate) fn answer(input: &Input) -> Result<Value, Failure> {
    let snapshot = Snapshot::build(&input.model_list)?;
    let cache = ScriptedCache::new(&input.cache);
    let moment = moment(input.now)?;
    let runtime = Builder::new_current_thread().build()?;
    match input.function {
        Function::Usage => usage_answer(input, &snapshot, &cache, moment, &runtime),
        Function::IoUsage => io_answer(input, &snapshot, &cache, moment, &runtime),
        Function::Remaining => remaining_answer(input, &snapshot, &cache, moment, &runtime),
        Function::Response => response_answer(input, &snapshot, &cache, moment, &runtime),
        Function::Filter => filter_answer(input, &snapshot),
    }
}

// Router.get_model_group_usage: the tpm and rpm sums plus the key lists the read asked for.
fn usage_answer(
    input: &Input,
    snapshot: &Snapshot,
    cache: &ScriptedCache,
    moment: SystemTime,
    runtime: &Runtime,
) -> Result<Value, Failure> {
    let group = group_of(input, "usage")?;
    let rows = snapshot.rows(group);
    let usage = runtime.block_on(get_model_group_usage(Some(rows.as_slice()), cache, moment))?;
    Ok(serde_json::to_value(UsageOutput {
        tpm: usage.tpm,
        rpm: usage.rpm,
        reads: cache.reads(),
    })?)
}

// Router.get_model_group_io_token_usage: the itpm and otpm sums over the same one read.
fn io_answer(
    input: &Input,
    snapshot: &Snapshot,
    cache: &ScriptedCache,
    moment: SystemTime,
    runtime: &Runtime,
) -> Result<Value, Failure> {
    let group = group_of(input, "io_usage")?;
    let rows = snapshot.rows(group);
    let usage = runtime.block_on(get_model_group_io_token_usage(
        Some(rows.as_slice()),
        cache,
        moment,
    ))?;
    Ok(serde_json::to_value(IoOutput {
        itpm: usage.itpm,
        otpm: usage.otpm,
        reads: cache.reads(),
    })?)
}

// Router.get_remaining_model_group_usage: the header map and the reads behind it, in write order.
fn remaining_answer(
    input: &Input,
    snapshot: &Snapshot,
    cache: &ScriptedCache,
    moment: SystemTime,
    runtime: &Runtime,
) -> Result<Value, Failure> {
    let group = group_of(input, "remaining")?;
    let rows = snapshot.rows(group);
    let metadata = FixedMetadata {
        info: group_info(input.group_info.as_ref()),
    };
    let headers = runtime.block_on(get_remaining_model_group_usage(
        group,
        Some(rows.as_slice()),
        &metadata,
        cache,
        moment,
    ))?;
    Ok(serde_json::to_value(RemainingOutput {
        headers,
        reads: cache.reads(),
    })?)
}

// Router.set_response_headers: the returned response body, hidden params and headers included.
fn response_answer(
    input: &Input,
    snapshot: &Snapshot,
    cache: &ScriptedCache,
    moment: SystemTime,
    runtime: &Runtime,
) -> Result<Value, Failure> {
    let metadata = FixedMetadata {
        info: group_info(input.group_info.as_ref()),
    };
    let rows = input.model_group.as_ref().map(|group| snapshot.rows(group));
    let returned = runtime.block_on(set_response_headers(ResponseHeadersRequest {
        response: response_body(input.response.as_ref()),
        model_group: input.model_group.as_deref(),
        request_kwargs: input.request_kwargs.as_ref(),
        model_list: rows.as_deref(),
        metadata: &metadata,
        cache,
        now: moment,
    }))?;
    Ok(serde_json::to_value(ResponseOutput {
        response: returned_body(returned)?,
        reads: cache.reads(),
    })?)
}

// Router._filter_deployments_by_model_access_groups: the ids of the candidates the grants leave.
fn filter_answer(input: &Input, snapshot: &Snapshot) -> Result<Value, Failure> {
    let filtered = filtered_candidates(input, snapshot)?;
    let ids = filtered
        .iter()
        .map(|deployment| deployment.id().unwrap_or_default().to_owned())
        .collect();
    Ok(serde_json::to_value(FilterOutput { ids })?)
}

fn filtered_candidates(input: &Input, snapshot: &Snapshot) -> Result<Vec<DeploymentSpec>, Failure> {
    let candidates = candidate_rows(input, snapshot)?;
    let metadata = input.metadata.as_ref().map(grants_of);
    let litellm_metadata = input.litellm_metadata.as_ref().map(grants_of);
    let no_inferred = |_model: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &snapshot.catalog,
        model_group_alias: &snapshot.aliases,
        routing_groups: &snapshot.groups,
        inferred_provider: &no_inferred,
    };
    Ok(filter_deployments_by_model_access_groups(
        AccessFilterRequest {
            model: group_of(input, "filter")?,
            deployments: &candidates,
            metadata: metadata.as_ref(),
            litellm_metadata: litellm_metadata.as_ref(),
            team_id: input.team_id.as_deref(),
        },
        &AccessGroups::default(),
        &lookup,
    )?)
}

// The request's held deployments: the whole model list, or the case's own indices into it.
fn candidate_rows(input: &Input, snapshot: &Snapshot) -> Result<Vec<DeploymentSpec>, Failure> {
    let Some(indices) = &input.candidates else {
        return Ok(snapshot.catalog.models.clone());
    };
    indices
        .iter()
        .map(|index| {
            snapshot
                .catalog
                .models
                .get(*index)
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

fn group_of<'input>(input: &'input Input, function: &'static str) -> Result<&'input str, Failure> {
    match input.model_group.as_deref() {
        Some(group) => Ok(group),
        None => Err(Failure::MissingGroup { function }),
    }
}

fn response_body(body: Option<&JsonObject>) -> Response {
    match body {
        Some(body) => Response::Object(body.clone()),
        None => Response::Absent,
    }
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
