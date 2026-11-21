use std::collections::BTreeMap;
use std::future::ready;
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
use betterlite_openai::fine_tuning::job::{FineTuneCreate, JobStatus};
use secrecy::SecretString;

use super::fixture::{NoSources, leaf_error, parse};
use crate::azure_resource_dispatch::fine_tuning::{Call, Dispatch, Outcome};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult,
};

const JOB_JSON: &str = r#"{"id":"ft_1","created_at":1,"error":null,"fine_tuned_model":null,"finished_at":null,"estimated_finish":null,"integrations":null,"metadata":null,"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3},"method":null,"model":"gpt-4o-mini","object":"fine_tuning.job","organization_id":"org-x","result_files":[],"seed":5,"status":"running","trained_tokens":null,"training_file":"file_a","validation_file":null}"#;
const PAGE_JSON: &str = r#"{"object":"list","data":[],"has_more":false}"#;
const CREATE_TEXT: &str = r#"{"model":"gpt-4o-mini","training_file":"file_a"}"#;

#[derive(Clone)]
enum Answer {
    Value(String),
    Failure(u16),
}

struct Fake {
    answer: Answer,
    calls: Mutex<Vec<String>>,
}

impl Fake {
    fn reply(&self, call: String) -> Result<Vec<u8>, SdkError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match &self.answer {
            Answer::Value(body) => Ok(body.clone().into_bytes()),
            Answer::Failure(status) => Err(SdkError {
                status: *status,
                error_type: Some(String::from("invalid_request_error")),
                param: None,
                code: None,
                message: String::from("the fixture failed"),
            }),
        }
    }
}

impl JobsApi for Fake {
    fn is_async(&self) -> bool {
        true
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

struct Materializer {
    answer: Answer,
}

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        _outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        Ok(Some(Box::new(Fake {
            answer: self.answer.clone(),
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

fn settings() -> ClientSettings {
    ClientSettings {
        api_key: Some(SecretString::from("key-1")),
        api_base: Some(String::from("https://fixture.test")),
        api_version: Some(String::from("2024-10-21")),
        timeout: None,
        max_retries: None,
        organization: None,
    }
}

// Every row runs the top-level dispatcher over the leaf's own connection, so the leaf opens the fake SDK
fn through(answer: Answer, call: Call<'_>) -> Result<AzureResourceResult, AzureResourceError> {
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = NoSources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let materializer = Materializer { answer };
    let connection = Connection {
        settings: settings(),
        client: None,
        is_async: true,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    };
    super::block_on(super::dispatch(AzureResourceOperation::FineTuning(
        Box::pin(ready(Dispatch { connection, call }.run())),
    )))
}

fn create_rows() {
    let mut data: FineTuneCreate = parse(CREATE_TEXT);
    let result = through(
        Answer::Value(String::from(JOB_JSON)),
        Call::Create(&mut data),
    )
    .expect("the create reaches its leaf");
    let AzureResourceResult::FineTuning(Outcome::Job(job)) = result else {
        panic!("the create returns a job");
    };
    assert_eq!(job.id, "ft_1");
    assert!(matches!(job.status, JobStatus::Running));
    let extra = data
        .extra
        .get("extra_body")
        .expect("the create gains an extra body");
    assert_eq!(extra.get("trainingType"), Some(&serde_json::json!(1_i32)));
}

fn list_rows() {
    let result = through(
        Answer::Value(String::from(PAGE_JSON)),
        Call::List {
            after: Some("ft_0"),
            limit: Some(10),
        },
    )
    .expect("the list reaches its leaf");
    let AzureResourceResult::FineTuning(Outcome::Page(page)) = result else {
        panic!("the list returns the SDK page");
    };
    assert_eq!(page, PAGE_JSON.as_bytes());
}

fn failure_rows() {
    let mut data: FineTuneCreate = parse(CREATE_TEXT);
    let error = leaf_error(through(Answer::Failure(400), Call::Create(&mut data)));
    assert!(matches!(
        error,
        AzureResourceError::FineTuning(AzureFineTuneError::Sdk {
            source: SdkError { status: 400, .. }
        })
    ));
}

pub(super) fn check() {
    create_rows();
    list_rows();
    failure_rows();
}
