#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_openai::fine_tuning::job::{FineTuneJob, JobStatus};
use secrecy::zeroize::Zeroize;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError,
};

use super::page::FineTuningPage;
use super::service::FineTuningRead;
use super::{
    ApiKeyText, FineTuningReadDispatch, FineTuningReadError, FineTuningReadInput,
    FineTuningReadOutcome, ReadOperation, ReadProvider, ReadTarget, ReadValues,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the provider answered {status}")]
struct FakeError {
    status: u16,
}

#[derive(Debug, Clone, PartialEq)]
enum RecordedOperation {
    List {
        after: Option<String>,
        limit: Option<i64>,
    },
    Retrieve {
        job_id: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedCall {
    is_async: bool,
    operation: RecordedOperation,
    timeout: FineTuningTimeout,
    max_retries: Option<i64>,
    api_base: Option<String>,
    api_key: Option<String>,
    api_version: Option<String>,
    organization: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum RecordedTarget {
    OpenAi(RecordedCall),
    Azure(RecordedCall),
}

#[derive(Debug, Default)]
struct State {
    calls: Vec<RecordedTarget>,
    reply: Vec<u8>,
    error: Option<FakeError>,
}

#[derive(Clone, Default)]
struct Fake {
    state: Arc<Mutex<State>>,
}

impl Fake {
    fn answering(reply: &[u8]) -> Self {
        let fake = Self::default();
        fake.state.lock().expect("the fake state").reply = reply.to_vec();
        fake
    }

    fn failing(error: FakeError) -> Self {
        let fake = Self::default();
        fake.state.lock().expect("the fake state").error = Some(error);
        fake
    }

    fn calls(&self) -> Vec<RecordedTarget> {
        self.state.lock().expect("the fake state").calls.clone()
    }
}

impl FineTuningReadDispatch for Fake {
    type Error = FakeError;

    async fn read(&self, target: ReadTarget<'_>) -> Result<Vec<u8>, Self::Error> {
        let mut state = self.state.lock().expect("the fake state");
        state.calls.push(record(&target));
        match &state.error {
            Some(error) => Err(error.clone()),
            None => Ok(state.reply.clone()),
        }
    }
}

fn record(target: &ReadTarget<'_>) -> RecordedTarget {
    match target {
        ReadTarget::OpenAi(call) => RecordedTarget::OpenAi(record_call(
            &call.values,
            Some(call.api_base.as_str()),
            call.api_key,
            call.api_version,
            call.organization,
        )),
        ReadTarget::Azure(call) => RecordedTarget::Azure(record_call(
            &call.values,
            call.api_base,
            call.api_key,
            call.api_version,
            call.organization,
        )),
    }
}

fn record_call(
    values: &ReadValues<'_>,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    api_version: Option<&str>,
    organization: Option<&str>,
) -> RecordedCall {
    let operation = match values.operation {
        ReadOperation::List { after, limit } => RecordedOperation::List {
            after: after.map(str::to_owned),
            limit,
        },
        ReadOperation::Retrieve { job_id } => RecordedOperation::Retrieve {
            job_id: job_id.to_owned(),
        },
    };
    RecordedCall {
        is_async: values.is_async,
        operation,
        timeout: *values.timeout,
        max_retries: values.max_retries,
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(|key| key.expose_secret().to_owned()),
        api_version: api_version.map(str::to_owned),
        organization: organization.map(str::to_owned),
    }
}

async fn run(
    fake: &Fake,
    settings: &ConfiguredSettings,
    input: &FineTuningReadInput,
) -> Result<FineTuningReadOutcome, FineTuningReadError<FakeError>> {
    FineTuningRead {
        dispatch: fake.clone(),
        settings: settings.clone(),
    }
    .run(input)
    .await
}

fn input(source: &str) -> FineTuningReadInput {
    serde_json::from_str(source).expect("a read call")
}

fn openai(calls: &[RecordedTarget]) -> RecordedCall {
    match calls.first().expect("one dispatch") {
        RecordedTarget::OpenAi(call) => call.clone(),
        RecordedTarget::Azure(_) => panic!("the fake was sent to azure"),
    }
}

fn azure(calls: &[RecordedTarget]) -> RecordedCall {
    match calls.first().expect("one dispatch") {
        RecordedTarget::Azure(call) => call.clone(),
        RecordedTarget::OpenAi(_) => panic!("the fake was sent to openai"),
    }
}

// every fine-tuning job field the fixtures share; each fixture changes what its call reads
const JOB: &str = r#"{"id": "ft_a", "object": "fine_tuning.job", "created_at": 1700000000, "model": "gpt-4o-mini-2024-07-18", "organization_id": "org_1", "result_files": ["file_r"], "status": "succeeded", "training_file": "file_1", "hyperparameters": {"n_epochs": 3}, "seed": 42}"#;

// the page carries the fields list doesn't normalize and one key the job shape never declares
const PAGE: &str = r#"{"object": "list", "has_more": true, "data": [{"id": "ft_a", "object": "fine_tuning.job", "created_at": 1700000000, "model": "gpt-4o-mini-2024-07-18", "organization_id": null, "result_files": null, "status": "canceling", "training_file": "file_1", "hyperparameters": {"n_epochs": 3}, "seed": 42, "method": {"type": "supervised", "hyperparameters": {"n_epochs": "auto"}}, "unknown_top": 1}]}"#;

// the azure answer differs in the fields its normalization reads
const AZURE_JOB: &str = r#"{"id": "ft_a", "object": "fine_tuning.job", "created_at": 1700000000, "model": "gpt-4o-mini", "organization_id": null, "result_files": null, "status": "canceled", "training_file": "file_1", "hyperparameters": {"n_epochs": 3}, "seed": 42}"#;

fn page() -> Vec<u8> {
    PAGE.as_bytes().to_vec()
}

fn job() -> Vec<u8> {
    JOB.as_bytes().to_vec()
}

fn azure_job() -> Vec<u8> {
    AZURE_JOB.as_bytes().to_vec()
}

// list and alist with no pagination leave after and limit null, and the default provider is openai
#[tokio::test]
async fn list_jobs_does_not_invent_limit() {
    let fake = Fake::answering(&page());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect("a listed page");
    assert!(matches!(outcome, FineTuningReadOutcome::Page(_)));
    let call = openai(&fake.calls());
    assert_eq!(
        call.operation,
        RecordedOperation::List {
            after: None,
            limit: None
        }
    );
    assert_eq!(call.api_base.as_deref(), Some("https://api.openai.com/v1"));
    assert_eq!(call.organization, None);
}

// a given empty after and a zero limit reach the provider as the caller wrote them
#[tokio::test]
async fn list_keeps_empty_after_and_zero_limit() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"after": "", "limit": 0}}"#),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&fake.calls()).operation,
        RecordedOperation::List {
            after: Some(String::new()),
            limit: Some(0)
        }
    );
}

// alist forwards the pagination it is given
#[tokio::test]
async fn alist_forwards_pagination() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"alist": {"after": "ft_prev", "limit": 7}}"#),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&fake.calls()).operation,
        RecordedOperation::List {
            after: Some("ft_prev".to_owned()),
            limit: Some(7)
        }
    );
}

// the page keeps its metadata and every declared job key, including the values list doesn't normalize
#[tokio::test]
async fn list_page_preserves_metadata_and_jobs() {
    let fake = Fake::answering(&page());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    let page_json = serde_json::to_value(page).expect("a page serializes");
    assert_eq!(page_json["object"], json!("list"));
    assert_eq!(page_json["has_more"], json!(true));
    let job = &page_json["data"][0];
    assert_eq!(job["status"], json!("canceling"));
    assert_eq!(job["organization_id"], json!(null));
    assert_eq!(job["training_file"], json!("file_1"));
    assert_eq!(job["unknown_top"], json!(1));
    assert_eq!(job["error"], json!(null));
    assert_eq!(
        job["hyperparameters"],
        json!({"batch_size": null, "learning_rate_multiplier": null, "n_epochs": 3})
    );
    assert_eq!(job["method"]["hyperparameters"]["n_epochs"], json!("auto"));
    assert_eq!(job.as_object().expect("a job object").len(), 20);
}

// a page with a missing object key omits it, while data and has_more stay present as null
#[tokio::test]
async fn list_page_keeps_absent_keys_absent() {
    let fake = Fake::answering(b"{}");
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    assert_eq!(
        serde_json::to_value(page).expect("a page serializes"),
        json!({"data": null, "has_more": null})
    );
}

// azure list entries come back exactly as the page wrote them; only retrieve normalizes a job
#[tokio::test]
async fn azure_list_entries_are_not_normalized() {
    let fake = Fake::answering(&page());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"provider": "azure", "api_version": "2024-08-01-preview"}}"#),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    let page_json = serde_json::to_value(page).expect("a page serializes");
    assert_eq!(page_json["data"][0]["status"], json!("canceling"));
    assert_eq!(page_json["data"][0]["organization_id"], json!(null));
    assert_eq!(page_json["data"][0]["result_files"], json!(null));
    assert_eq!(
        azure(&fake.calls()).api_version.as_deref(),
        Some("2024-08-01-preview")
    );
}

// retrieve hands back the decoded job with every provider field it wrote
#[tokio::test]
async fn retrieve_job_preserves_provider_result() {
    let fake = Fake::answering(&job());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"retrieve": {"fine_tuning_job_id": "ft_a"}}"#),
    )
    .await
    .expect("a retrieved job");
    let FineTuningReadOutcome::Job(job) = outcome else {
        panic!("retrieve answered a page");
    };
    let FineTuneJob {
        training_file,
        status,
        seed,
        ..
    } = *job;
    assert_eq!(training_file, "file_1");
    assert_eq!(status, JobStatus::Succeeded);
    assert_eq!(seed, Some(42));
    assert!(!openai(&fake.calls()).is_async);
    assert_eq!(
        openai(&fake.calls()).operation,
        RecordedOperation::Retrieve {
            job_id: "ft_a".to_owned()
        }
    );
}

// azure retrieve runs B518's azure normalization: statuses map and null org and files fill in
#[tokio::test]
async fn azure_retrieve_normalizes_job() {
    let fake = Fake::answering(&azure_job());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(
            r#"{"aretrieve": {"fine_tuning_job_id": "ft_a", "provider": "azure", "api_version": "2024-08-01-preview"}}"#,
        ),
    )
    .await
    .expect("a retrieved job");
    let FineTuningReadOutcome::Job(job) = outcome else {
        panic!("retrieve answered a page");
    };
    assert_eq!(job.status, JobStatus::Cancelled);
    assert_eq!(job.organization_id, "");
    assert!(job.result_files.is_empty());
    assert!(azure(&fake.calls()).is_async);
}

// the loaded settings every credential test starts from; each test drops or keeps what it reads
fn settings() -> ConfiguredSettings {
    ConfiguredSettings {
        api_base: Some("http://settings/v1".to_owned()),
        api_version: None,
        api_key: Some(SecretString::from("sk-settings".to_owned())),
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: Some("org-settings".to_owned()),
        vertex_project: None,
        vertex_location: None,
        openai_base_url: Some("http://env-base/v1".to_owned()),
        openai_api_base: None,
        openai_api_key: Some(SecretString::from("sk-env".to_owned())),
        openai_organization: Some("org-env".to_owned()),
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

// the openai branch reads request credentials before the loaded settings
#[tokio::test]
async fn read_settings_and_client_mode() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &settings(),
        &input(
            r#"{"list": {"api_base": "http://request/v1", "api_key": "sk-request", "organization": "org-request"}}"#,
        ),
    )
    .await
    .expect("a listed page");
    let call = openai(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://request/v1"));
    assert_eq!(call.api_key.as_deref(), Some("sk-request"));
    assert_eq!(call.organization.as_deref(), Some("org-request"));
}

// without request credentials the settings answer, and without those the environment does
#[tokio::test]
async fn read_falls_back_to_settings_and_environment() {
    let fake = Fake::answering(&page());
    run(&fake, &settings(), &input(r#"{"list": {}}"#))
        .await
        .expect("a listed page");
    let call = openai(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://settings/v1"));
    assert_eq!(call.api_key.as_deref(), Some("sk-settings"));
    assert_eq!(call.organization.as_deref(), Some("org-settings"));

    let mut environment = settings();
    environment.api_base = None;
    environment.api_key = None;
    environment.organization = None;
    let fallback = Fake::answering(&page());
    run(&fallback, &environment, &input(r#"{"list": {}}"#))
        .await
        .expect("a listed page");
    let env_call = openai(&fallback.calls());
    assert_eq!(env_call.api_base.as_deref(), Some("http://env-base/v1"));
    assert_eq!(env_call.api_key.as_deref(), Some("sk-env"));
    assert_eq!(env_call.organization.as_deref(), Some("org-env"));
}

// the azure settings differ from the openai ones in every name the branch reads
fn azure_settings() -> ConfiguredSettings {
    ConfiguredSettings {
        api_base: Some("http://settings/v1".to_owned()),
        api_version: Some("2024-01-01".to_owned()),
        api_key: Some(SecretString::from("sk-settings".to_owned())),
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: Some("org-settings".to_owned()),
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: Some("http://azure-env/v1".to_owned()),
        azure_api_version: Some("2024-06-01".to_owned()),
        azure_openai_api_key: Some(SecretString::from("sk-azure-env".to_owned())),
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

// azure reads the request first, and its organization has no setting fallback
#[tokio::test]
async fn azure_settings_chain_and_request_organization() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &azure_settings(),
        &input(r#"{"list": {"provider": "azure", "organization": "org-request"}}"#),
    )
    .await
    .expect("a listed page");
    let call = azure(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://settings/v1"));
    assert_eq!(call.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(call.api_key.as_deref(), Some("sk-settings"));
    assert_eq!(call.organization.as_deref(), Some("org-request"));
}

// without request or setting values azure reads the environment's azure names
#[tokio::test]
async fn azure_falls_back_to_environment() {
    let mut environment = azure_settings();
    environment.api_base = None;
    environment.api_version = None;
    environment.api_key = None;
    let fake = Fake::answering(&page());
    run(
        &fake,
        &environment,
        &input(r#"{"list": {"provider": "azure", "organization": "org-request"}}"#),
    )
    .await
    .expect("a listed page");
    let call = azure(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://azure-env/v1"));
    assert_eq!(call.api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(call.api_key.as_deref(), Some("sk-azure-env"));
    assert_eq!(call.organization.as_deref(), Some("org-request"));
}

async fn list_timeout(source: &str) -> FineTuningTimeout {
    let fake = Fake::answering(&page());
    run(&fake, &ConfiguredSettings::default(), &input(source))
        .await
        .expect("a listed page");
    openai(&fake.calls()).timeout
}

// timeout then request_timeout then 600, with a zero or empty value falling through
#[tokio::test]
async fn timeout_chain_and_structured_passthrough() {
    assert_eq!(
        list_timeout(r#"{"list": {}}"#).await,
        FineTuningTimeout::Seconds(600.0)
    );
    assert_eq!(
        list_timeout(r#"{"list": {"timeout": 0, "request_timeout": 9}}"#).await,
        FineTuningTimeout::Seconds(9.0)
    );
    assert_eq!(
        list_timeout(r#"{"list": {"timeout": "", "request_timeout": 9}}"#).await,
        FineTuningTimeout::Seconds(9.0)
    );
    assert_eq!(
        list_timeout(r#"{"list": {"timeout": 5}}"#).await,
        FineTuningTimeout::Seconds(5.0)
    );
    assert_eq!(
        list_timeout(r#"{"list": {"timeout": "5", "request_timeout": 9}}"#).await,
        FineTuningTimeout::Seconds(5.0)
    );
    assert_eq!(
        list_timeout(r#"{"list": {"request_timeout": "7.5"}}"#).await,
        FineTuningTimeout::Seconds(7.5)
    );
}

// a structured timeout passes through as the four numbers the caller wrote
#[tokio::test]
async fn structured_timeout_passes_through() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &ConfiguredSettings::default(),
        &input(
            r#"{"list": {"provider": "azure", "timeout": {"connect": 1.0, "read": 2.0, "write": 3.0, "pool": 4.0}}}"#,
        ),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        azure(&fake.calls()).timeout,
        FineTuningTimeout::Structured(StructuredTimeout {
            connect: Some(1.0),
            read: Some(2.0),
            write: Some(3.0),
            pool: Some(4.0)
        })
    );
}

// list is the sync form and alist sets the async flag; extras ride along and never reach the target
#[tokio::test]
async fn async_flags_and_unused_extras() {
    let sync = Fake::answering(&page());
    run(
        &sync,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect("a listed page");
    assert!(!openai(&sync.calls()).is_async);

    let asynchronous = Fake::answering(&page());
    run(
        &asynchronous,
        &ConfiguredSettings::default(),
        &input(r#"{"alist": {}}"#),
    )
    .await
    .expect("a listed page");
    assert!(openai(&asynchronous.calls()).is_async);

    let extras = Fake::answering(&page());
    run(
        &extras,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"extra_headers": {"X-Ignored": "x"}, "extra_body": {"limit": 99}}}"#),
    )
    .await
    .expect("a listed page");
    assert_eq!(openai(&extras.calls()), openai(&sync.calls()));
}

// max_retries and an openai api_version ride to the call; the openai client sends no version
#[tokio::test]
async fn openai_keeps_max_retries_and_api_version() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"max_retries": 3, "api_version": "2023-01-01"}}"#),
    )
    .await
    .expect("a listed page");
    let call = openai(&fake.calls());
    assert_eq!(call.max_retries, Some(3));
    assert_eq!(call.api_version.as_deref(), Some("2023-01-01"));
}

// an unsupported provider answers its own 400 without a call, and list keeps create_batch's name
#[tokio::test]
async fn unsupported_read_provider() {
    let fake = Fake::default();
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"provider": "vertex_ai"}}"#),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support vertex_ai for 'create_batch'. Only 'openai' is supported."
    );

    let retrieve = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"retrieve": {"fine_tuning_job_id": "ft_a", "provider": "vertex_ai"}}"#),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        retrieve.to_string(),
        "LiteLLM doesn't support vertex_ai for 'retrieve_fine_tuning_job'. Only 'openai' and 'azure' are supported."
    );
    assert!(fake.calls().is_empty());
}

// an explicit null and an unknown name take the branch they reached, which keeps Python's text
#[tokio::test]
async fn unsupported_provider_null_and_unknown() {
    let fake = Fake::default();
    let null = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"provider": null}}"#),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        null.to_string(),
        "LiteLLM doesn't support None for 'create_batch'. Only 'openai' is supported."
    );

    let unknown = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"retrieve": {"fine_tuning_job_id": "ft_a", "provider": "wat"}}"#),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        unknown.to_string(),
        "LiteLLM doesn't support wat for 'retrieve_fine_tuning_job'. Only 'openai' and 'azure' are supported."
    );
}

// an unparsable timeout is a plain ValueError, raised before any provider work
#[tokio::test]
async fn invalid_timeout_fails_before_dispatch() {
    let fake = Fake::answering(&page());
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {"timeout": "bad"}}"#),
    )
    .await
    .expect_err("an invalid timeout");
    assert!(matches!(
        error,
        FineTuningReadError::Timeout(TimeoutError::InvalidNumber { .. })
    ));
    assert!(fake.calls().is_empty());
}

// a provider error keeps the fields the SDK projected, and no page is decoded
#[tokio::test]
async fn dispatch_errors_propagate() {
    let fake = Fake::failing(FakeError { status: 404 });
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect_err("a provider error");
    let FineTuningReadError::Dispatch(source) = error else {
        panic!("the provider error was not kept");
    };
    assert_eq!(source.status, 404);
}

// a provider answer that is not JSON is reported as a page error after the dispatch
#[tokio::test]
async fn page_json_errors_are_reported() {
    let fake = Fake::answering(b"not json");
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(r#"{"list": {}}"#),
    )
    .await
    .expect_err("an invalid page");
    assert!(matches!(error, FineTuningReadError::Page { .. }));
    assert_eq!(fake.calls().len(), 1);
}

// the SecretBox wipes the key through this impl, so a no-op here would leave the text readable
#[test]
fn api_key_text_zeroizes() {
    let mut key = ApiKeyText("sk-request".to_owned());
    key.zeroize();
    assert!(key.0.bytes().all(|byte| byte == 0));
}

// provider is a literal in the source, so an absent one takes openai and an explicit null stays null
#[test]
fn provider_defaults_only_when_absent() {
    let FineTuningReadInput::List(absent) = input(r#"{"list": {}}"#) else {
        panic!("a list call");
    };
    assert_eq!(absent.provider, Some(ReadProvider("openai".to_owned())));

    let FineTuningReadInput::List(null) = input(r#"{"list": {"provider": null}}"#) else {
        panic!("a list call");
    };
    assert_eq!(null.provider, None);
}
