use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, ClientOutcome, SessionKind,
};
use betterlite_azure::fine_tuning::client::{ClientMaterializer, Connection};
use betterlite_azure::fine_tuning::error::AzureFineTuneError;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::fine_tuning::client::{BuildError, ClientSettings, JobsApi, SdkError};
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch, fine_tuning,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::support::{NoSources, OTHER, failure, failure_with_status, runtime};

const CREATE_TEXT: &str = r#"{"model":"gpt-4o-mini","training_file":"file_a"}"#;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) steps: Vec<Step>,
}

// One B542 call per step over an Azure endpoint and version the case supplies
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Step {
    Create(Fixture),
    List(Fixture),
    Retrieve(Fixture),
    Cancel(Fixture),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Fixture {
    pub(super) api_base: String,
    pub(super) api_version: String,
    pub(super) api_key: String,
    pub(super) status: u16,
    pub(super) body: String,
    #[serde(default)]
    pub(super) job_id: String,
    #[serde(default)]
    pub(super) after: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<i64>,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let mut answers = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        let (kind, value) = answer(step)?;
        answers.push(json!({"kind": kind, "value": value}));
    }
    Ok(json!({"steps": answers}))
}

fn answer(step: &Step) -> Result<(&'static str, Value), Box<HelperError>> {
    let fixture = fixture(step);
    let mut data: FineTuneCreate = serde_json::from_str(CREATE_TEXT).map_err(failure)?;
    let call = match step {
        Step::Create(_) => fine_tuning::Call::Create(&mut data),
        Step::List(_) => fine_tuning::Call::List {
            after: fixture.after.as_deref(),
            limit: fixture.limit,
        },
        Step::Retrieve(_) => fine_tuning::Call::Retrieve(&fixture.job_id),
        Step::Cancel(_) => fine_tuning::Call::Cancel(&fixture.job_id),
    };
    let result = through(fixture, call)?;
    let kind = kind(step);
    let value = match result {
        AzureResourceResult::FineTuning(fine_tuning::Outcome::Job(job)) => {
            job_value(step, &job, &data)
        }
        AzureResourceResult::FineTuning(fine_tuning::Outcome::Page(page)) => page_value(&page)?,
        _ => return Err(failure(OTHER)),
    };
    Ok((kind, value))
}

const fn kind(step: &Step) -> &'static str {
    match step {
        Step::Create(_) => "create",
        Step::List(_) => "list",
        Step::Retrieve(_) => "retrieve",
        Step::Cancel(_) => "cancel",
    }
}

const fn fixture(step: &Step) -> &Fixture {
    match step {
        Step::Create(value) | Step::List(value) | Step::Retrieve(value) | Step::Cancel(value) => {
            value
        }
    }
}

fn job_value(step: &Step, job: &FineTuneJob, data: &FineTuneCreate) -> Value {
    let status = serde_json::to_value(job.status).unwrap_or(Value::Null);
    if matches!(step, Step::Create(_)) {
        return json!({"id": job.id, "status": status, "training_type": training_type(data)});
    }
    json!({"id": job.id, "status": status})
}

fn training_type(data: &FineTuneCreate) -> Value {
    data.extra
        .get("extra_body")
        .and_then(|body| body.get("trainingType"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn page_value(page: &[u8]) -> Result<Value, Box<HelperError>> {
    let parsed: Value = serde_json::from_slice(page).map_err(failure)?;
    let count = parsed
        .get("data")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let has_more = parsed
        .get("has_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(json!({"count": count, "has_more": has_more}))
}

fn through(
    fixture: &Fixture,
    call: fine_tuning::Call<'_>,
) -> Result<AzureResourceResult, Box<HelperError>> {
    let runtime = runtime()?;
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = NoSources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let materializer = Materializer {
        body: fixture.body.clone(),
        failure: (fixture.status >= 400).then_some(fixture.status),
    };
    let connection = Connection {
        settings: ClientSettings {
            api_key: Some(SecretString::from(fixture.api_key.clone())),
            api_base: Some(fixture.api_base.clone()),
            api_version: Some(fixture.api_version.clone()),
            timeout: None,
            max_retries: None,
            organization: None,
        },
        client: None,
        is_async: false,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::FineTuning(
        fine_tuning::dispatch(fine_tuning::Dispatch { connection, call }),
    )));
    match result {
        Ok(outcome) => Ok(outcome),
        Err(AzureResourceError::FineTuning(AzureFineTuneError::Sdk { source })) => {
            Err(failure_with_status(source.status, source.message))
        }
        Err(error) => Err(failure(error)),
    }
}

struct Materializer {
    body: String,
    failure: Option<u16>,
}

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        _outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        Ok(Some(Box::new(Fake {
            body: self.body.clone(),
            failure: self.failure,
            calls: Mutex::new(Vec::new()),
        })))
    }
}

#[derive(Default)]
struct Cache;

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

struct Fake {
    body: String,
    failure: Option<u16>,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn reply(&self, call: String) -> Result<Vec<u8>, SdkError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.failure {
            Some(status) => Err(SdkError {
                status,
                error_type: None,
                param: None,
                code: None,
                message: String::from("the fixture failed"),
            }),
            None => Ok(self.body.clone().into_bytes()),
        }
    }
}

impl JobsApi for Fake {
    fn is_async(&self) -> bool {
        false
    }

    fn create(&self, _data: &FineTuneCreate) -> Result<Vec<u8>, SdkError> {
        self.reply(String::from("create"))
    }

    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Result<Vec<u8>, SdkError> {
        self.reply(format!("list:{after:?}:{limit:?}"))
    }

    fn retrieve(&self, job_id: &str) -> Result<Vec<u8>, SdkError> {
        self.reply(format!("retrieve:{job_id}"))
    }

    fn cancel(&self, job_id: &str) -> Result<Vec<u8>, SdkError> {
        self.reply(format!("cancel:{job_id}"))
    }
}
