use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use thiserror::Error;

use crate::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError,
};

use super::page::FineTuningPage;
use super::{
    FineTuningRead, FineTuningReadDispatch, FineTuningReadError, FineTuningReadInput,
    FineTuningReadOutcome, ReadOperation, ReadTarget, ReadValues,
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

    fn read(
        &self,
        target: ReadTarget<'_>,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        async move {
            let mut state = self.state.lock().expect("the fake state");
            state.calls.push(record(&target));
            match &state.error {
                Some(error) => Err(error.clone()),
                None => Ok(state.reply.clone()),
            }
        }
    }
}

fn record(target: &ReadTarget<'_>) -> RecordedTarget {
    match target {
        ReadTarget::OpenAi(call) => RecordedTarget::OpenAi(record_call(
            &call.values,
            Some(&call.api_base),
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
    api_key: Option<&secrecy::SecretString>,
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

fn input(value: Value) -> FineTuningReadInput {
    serde_json::from_value(value).expect("a read call")
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
fn job_json() -> Value {
    json!({
        "id": "ft_a",
        "object": "fine_tuning.job",
        "created_at": 1700000000,
        "model": "gpt-4o-mini-2024-07-18",
        "organization_id": "org_1",
        "result_files": ["file_r"],
        "status": "succeeded",
        "training_file": "file_1",
        "hyperparameters": {"n_epochs": 3},
        "seed": 42
    })
}

fn page() -> Vec<u8> {
    let mut entry = job_json();
    entry["status"] = json!("canceling");
    entry["organization_id"] = Value::Null;
    entry["result_files"] = Value::Null;
    entry["method"] = json!({"type": "supervised", "hyperparameters": {"n_epochs": "auto"}});
    entry["unknown_top"] = json!(1);
    let envelope = json!({"object": "list", "has_more": true, "data": [entry]});
    envelope.to_string().into_bytes()
}

fn job() -> Vec<u8> {
    let entry = job_json();
    entry.to_string().into_bytes()
}

// the azure answer differs in the fields its normalization reads
fn azure_job() -> Vec<u8> {
    let mut entry = job_json();
    entry["model"] = json!("gpt-4o-mini");
    entry["organization_id"] = Value::Null;
    entry["result_files"] = Value::Null;
    entry["status"] = json!("canceled");
    entry.to_string().into_bytes()
}

// list and alist with no pagination leave after and limit null, and the default provider is openai
#[tokio::test]
async fn list_jobs_does_not_invent_limit() {
    let fake = Fake::answering(&page());
    let outcome = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(json!({"list": {}})),
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

    let literal = Fake::answering(&page());
    let outcome = run(
        &literal,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"after": "", "limit": 0}})),
    )
    .await
    .expect("a listed page");
    assert!(matches!(outcome, FineTuningReadOutcome::Page(_)));
    assert_eq!(
        openai(&literal.calls()).operation,
        RecordedOperation::List {
            after: Some(String::new()),
            limit: Some(0)
        }
    );

    let paged = Fake::answering(&page());
    run(
        &paged,
        &ConfiguredSettings::default(),
        &input(json!({"alist": {"after": "ft_prev", "limit": 7}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&paged.calls()).operation,
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
        &input(json!({"list": {}})),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    assert_eq!(page["object"], json!("list"));
    assert_eq!(page["has_more"], json!(true));
    let job = &page["data"][0];
    assert_eq!(job["status"], json!("canceling"));
    assert_eq!(job["organization_id"], Value::Null);
    assert_eq!(job["training_file"], json!("file_1"));
    assert_eq!(job["unknown_top"], json!(1));
    assert_eq!(job["error"], Value::Null);
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
        &input(json!({"list": {}})),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    assert_eq!(page, json!({"data": null, "has_more": null}));
}

// azure list entries come back exactly as the page wrote them; only retrieve normalizes a job
#[tokio::test]
async fn azure_list_entries_are_not_normalized() {
    let fake = Fake::answering(&page());
    let settings = ConfiguredSettings::default();
    let outcome = run(
        &fake,
        &settings,
        &input(json!({"list": {"provider": "azure", "api_version": "2024-08-01-preview"}})),
    )
    .await
    .expect("a listed page");
    let FineTuningReadOutcome::Page(FineTuningPage(page)) = outcome else {
        panic!("list answered a job");
    };
    assert_eq!(page["data"][0]["status"], json!("canceling"));
    assert_eq!(page["data"][0]["organization_id"], Value::Null);
    assert_eq!(page["data"][0]["result_files"], Value::Null);
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
        &input(json!({"retrieve": {"fine_tuning_job_id": "ft_a"}})),
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
    assert_eq!(format!("{status:?}"), "Succeeded");
    assert_eq!(seed, Some(42));
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
        &input(json!({"aretrieve": {
            "fine_tuning_job_id": "ft_a",
            "provider": "azure",
            "api_version": "2024-08-01-preview"
        }})),
    )
    .await
    .expect("a retrieved job");
    let FineTuningReadOutcome::Job(job) = outcome else {
        panic!("retrieve answered a page");
    };
    assert_eq!(format!("{:?}", job.status), "Cancelled");
    assert_eq!(job.organization_id, "");
    assert_eq!(job.result_files, Vec::<String>::new());
    assert!(azure(&fake.calls()).is_async);
}

// the openai branch reads request credentials, then the loaded settings, then the default base
#[tokio::test]
async fn read_settings_and_client_mode() {
    let mut settings = ConfiguredSettings {
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
    };
    let fake = Fake::answering(&page());
    run(
        &fake,
        &settings,
        &input(json!({"list": {
            "api_base": "http://request/v1",
            "api_key": "sk-request",
            "organization": "org-request"
        }})),
    )
    .await
    .expect("a listed page");
    let call = openai(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://request/v1"));
    assert_eq!(call.api_key.as_deref(), Some("sk-request"));
    assert_eq!(call.organization.as_deref(), Some("org-request"));

    let fallback = Fake::answering(&page());
    run(&fallback, &settings, &input(json!({"list": {}})))
        .await
        .expect("a listed page");
    let call = openai(&fallback.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://settings/v1"));
    assert_eq!(call.api_key.as_deref(), Some("sk-settings"));
    assert_eq!(call.organization.as_deref(), Some("org-settings"));

    settings.api_base = None;
    settings.api_key = None;
    settings.organization = None;
    let environment = Fake::answering(&page());
    run(&environment, &settings, &input(json!({"list": {}})))
        .await
        .expect("a listed page");
    let call = openai(&environment.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://env-base/v1"));
    assert_eq!(call.api_key.as_deref(), Some("sk-env"));
    assert_eq!(call.organization.as_deref(), Some("org-env"));
}

// the azure branch never reads the openai names, and its organization has no setting fallback
#[tokio::test]
async fn azure_settings_chain_and_request_organization() {
    let settings = ConfiguredSettings {
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
    };
    let fake = Fake::answering(&page());
    run(
        &fake,
        &settings,
        &input(json!({"list": {"provider": "azure"}})),
    )
    .await
    .expect("a listed page");
    let call = azure(&fake.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://settings/v1"));
    assert_eq!(call.api_version.as_deref(), Some("2024-01-01"));
    assert_eq!(call.api_key.as_deref(), Some("sk-settings"));
    assert_eq!(call.organization, None);

    let environment = Fake::answering(&page());
    let settings = ConfiguredSettings {
        api_base: None,
        api_version: None,
        api_key: None,
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
    };
    run(
        &environment,
        &settings,
        &input(json!({"list": {"provider": "azure", "organization": "org-request"}})),
    )
    .await
    .expect("a listed page");
    let call = azure(&environment.calls());
    assert_eq!(call.api_base.as_deref(), Some("http://azure-env/v1"));
    assert_eq!(call.api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(call.api_key.as_deref(), Some("sk-azure-env"));
    assert_eq!(call.organization.as_deref(), Some("org-request"));
}

// timeout then request_timeout then 600, with a zero or empty value falling through
#[tokio::test]
async fn timeout_chain_and_structured_passthrough() {
    let fake = Fake::answering(&page());
    run(
        &fake,
        &ConfiguredSettings::default(),
        &input(json!({"list": {}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&fake.calls()).timeout,
        FineTuningTimeout::Seconds(600.0)
    );

    let zero = Fake::answering(&page());
    run(
        &zero,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"timeout": 0, "request_timeout": 9}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&zero.calls()).timeout,
        FineTuningTimeout::Seconds(9.0)
    );

    let empty = Fake::answering(&page());
    run(
        &empty,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"timeout": "", "request_timeout": 9}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&empty.calls()).timeout,
        FineTuningTimeout::Seconds(9.0)
    );

    let text = Fake::answering(&page());
    run(
        &text,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"request_timeout": "7.5"}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&text.calls()).timeout,
        FineTuningTimeout::Seconds(7.5)
    );

    let alone = Fake::answering(&page());
    run(
        &alone,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"timeout": 5}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&alone.calls()).timeout,
        FineTuningTimeout::Seconds(5.0)
    );

    let chosen = Fake::answering(&page());
    run(
        &chosen,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"timeout": "5", "request_timeout": 9}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        openai(&chosen.calls()).timeout,
        FineTuningTimeout::Seconds(5.0)
    );

    let structured = Fake::answering(&page());
    run(
        &structured,
        &ConfiguredSettings::default(),
        &input(json!({"list": {
            "provider": "azure",
            "timeout": {"connect": 1.0, "read": 2.0, "write": 3.0, "pool": 4.0}
        }})),
    )
    .await
    .expect("a listed page");
    assert_eq!(
        azure(&structured.calls()).timeout,
        FineTuningTimeout::Structured(StructuredTimeout {
            connect: Some(1.0),
            read: Some(2.0),
            write: Some(3.0),
            pool: Some(4.0)
        })
    );
}

// the four functions set the async flag, and extras ride along without reaching the target
#[tokio::test]
async fn async_flags_and_unused_extras() {
    let list = Fake::answering(&page());
    run(
        &list,
        &ConfiguredSettings::default(),
        &input(json!({"list": {}})),
    )
    .await
    .expect("a listed page");
    assert!(!openai(&list.calls()).is_async);

    let alist = Fake::answering(&page());
    run(
        &alist,
        &ConfiguredSettings::default(),
        &input(json!({"alist": {}})),
    )
    .await
    .expect("a listed page");
    assert!(openai(&alist.calls()).is_async);

    let extras = Fake::answering(&page());
    run(
        &extras,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"extra_headers": {"X-Ignored": "x"}, "extra_body": {"limit": 99}}})),
    )
    .await
    .expect("a listed page");
    assert_eq!(openai(&extras.calls()), openai(&list.calls()));

    let retrieve = Fake::answering(&job());
    run(
        &retrieve,
        &ConfiguredSettings::default(),
        &input(json!({"retrieve": {"fine_tuning_job_id": "ft_a"}})),
    )
    .await
    .expect("a retrieved job");
    assert!(!openai(&retrieve.calls()).is_async);

    let aretrieve = Fake::answering(&job());
    run(
        &aretrieve,
        &ConfiguredSettings::default(),
        &input(json!({"aretrieve": {"fine_tuning_job_id": "ft_a"}})),
    )
    .await
    .expect("a retrieved job");
    assert!(openai(&aretrieve.calls()).is_async);
    assert_eq!(
        openai(&aretrieve.calls()).operation,
        RecordedOperation::Retrieve {
            job_id: "ft_a".to_owned()
        }
    );
}

// vertex_ai, an unknown name and an explicit null all answer the provider's own 400 without a call
#[tokio::test]
async fn unsupported_read_provider() {
    let fake = Fake::default();
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"provider": "vertex_ai"}})),
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
        &input(json!({"retrieve": {"fine_tuning_job_id": "ft_a", "provider": "vertex_ai"}})),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        retrieve.to_string(),
        "LiteLLM doesn't support vertex_ai for 'retrieve_fine_tuning_job'. Only 'openai' and 'azure' are supported."
    );

    let null = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"provider": null}})),
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
        &input(json!({"retrieve": {"fine_tuning_job_id": "ft_a", "provider": "wat"}})),
    )
    .await
    .expect_err("an unsupported provider");
    assert_eq!(
        unknown.to_string(),
        "LiteLLM doesn't support wat for 'retrieve_fine_tuning_job'. Only 'openai' and 'azure' are supported."
    );
    assert!(fake.calls().is_empty());
}

// an unparsable timeout is a plain ValueError, raised before any provider work
#[tokio::test]
async fn invalid_timeout_fails_before_dispatch() {
    let fake = Fake::answering(&page());
    let error = run(
        &fake,
        &ConfiguredSettings::default(),
        &input(json!({"list": {"timeout": "bad"}})),
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
        &input(json!({"list": {}})),
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
        &input(json!({"list": {}})),
    )
    .await
    .expect_err("an invalid page");
    assert!(matches!(error, FineTuningReadError::Page { .. }));
    assert_eq!(fake.calls().len(), 1);
}

// provider is a literal in the source, so an absent one takes openai and an explicit null stays null
#[tokio::test]
async fn provider_defaults_only_when_absent() {
    let absent: FineTuningReadInput = serde_json::from_value(json!({"list": {}})).expect("a call");
    let FineTuningReadInput::List(call) = absent else {
        panic!("a list call");
    };
    assert_eq!(
        call.provider,
        Some(super::ReadProvider("openai".to_owned()))
    );

    let null: FineTuningReadInput =
        serde_json::from_value(json!({"list": {"provider": null}})).expect("a call");
    let FineTuningReadInput::List(call) = null else {
        panic!("a list call");
    };
    assert_eq!(call.provider, None);
}
