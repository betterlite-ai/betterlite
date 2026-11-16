use std::collections::BTreeMap;
use std::mem;
use std::sync::{Arc, PoisonError};

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::projection::SessionKind;
use betterlite_azure::fine_tuning::client::Connection;
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_azure::fine_tuning::{
    create_fine_tuning_job, list_fine_tuning_jobs, request_fine_tuning_job,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::fine_tuning::JobRequest;
use betterlite_openai::fine_tuning::client::{ClientSettings, JobsApi};
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use serde::Deserialize;
use serde_json::{Value, json};

use super::fixture::{Cache, Log, Materializer, Planned, Recorded, Reply, Sources, planned_reply};
use super::reports::{projection, raised};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) steps: Vec<Step>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Project,
    Create,
    List,
    Retrieve,
    Cancel,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuilderMode {
    #[default]
    Default,
    None,
    WrongAsync,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    action: Action,
    args: ClientSettings,
    #[serde(default)]
    is_async: bool,
    client: Option<String>,
    #[serde(default)]
    builder: BuilderMode,
    data: Option<FineTuneCreate>,
    job_id: Option<String>,
    after: Option<String>,
    limit: Option<i64>,
    reply: Option<Planned>,
}

fn perform(
    step: &mut Step,
    mut connection: Connection<'_, '_, Materializer>,
) -> Result<Value, AzureFineTuneError> {
    let job_id = step.job_id.clone().unwrap_or_default();
    match step.action {
        Action::Project => connection
            .get_openai_client()
            .map(|outcome| projection(&outcome)),
        Action::Create => {
            let data = step.data.get_or_insert_default();
            create_fine_tuning_job(connection, data).map(|job| json!(job))
        }
        Action::List => {
            let page = list_fine_tuning_jobs(connection, step.after.as_deref(), step.limit)?;
            serde_json::from_slice::<Value>(&page).map_err(AzureFineTuneError::from)
        }
        Action::Retrieve => request_fine_tuning_job(connection, &JobRequest::Retrieve(&job_id))
            .map(|job| json!(job)),
        Action::Cancel => {
            request_fine_tuning_job(connection, &JobRequest::Cancel(&job_id)).map(|job| json!(job))
        }
    }
}

fn client_for(step: &mut Step, reply: &Reply, supplied_calls: &Log) -> Option<Box<dyn JobsApi>> {
    step.client.take().map(|kind| -> Box<dyn JobsApi> {
        Box::new(Recorded::new(
            kind.contains("async"),
            reply.clone(),
            Arc::clone(supplied_calls),
        ))
    })
}

pub(crate) fn run_step(mut step: Step) -> Value {
    let (calls, supplied_calls) = (Log::default(), Log::default());
    let reply = planned_reply(step.reply.as_ref());
    let client = client_for(&mut step, &reply, &supplied_calls);
    let materializer = Materializer {
        mode: step.builder,
        reply,
        calls: Arc::clone(&calls),
    };
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = Sources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let connection = Connection {
        settings: mem::take(&mut step.args),
        client,
        is_async: step.is_async,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    };
    let outcome = match perform(&mut step, connection) {
        Ok(value) => json!({ "returned": value }),
        Err(error) => raised(&error),
    };
    // LiteLLM changes the caller's create data in place, even when the client never gets built
    json!({
        "data": step.data,
        "outcome": outcome,
        "calls": calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "supplied_calls": supplied_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    })
}
