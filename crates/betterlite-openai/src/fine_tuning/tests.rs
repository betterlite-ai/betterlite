#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::client::{
    BuildError, ClientArgs, ClientBuilder, ClientSettings, Connection, JobsApi, SdkError,
};
use super::error::FineTuneError;
use super::job::{FineTuneCreate, decode_fine_tune_job};
use super::normalize::normalize_fine_tuning_job_dict;
use super::{
    cancel_fine_tuning_job, create_fine_tuning_job, list_fine_tuning_jobs, retrieve_fine_tuning_job,
};

type Log = Arc<Mutex<Vec<Value>>>;

#[derive(Deserialize)]
struct Reply {
    status: u16,
    body: Value,
}

#[derive(Deserialize)]
struct Step {
    action: String,
    #[serde(default)]
    is_async: bool,
    client: Option<String>,
    builder: Option<String>,
    reply: Option<Reply>,
    data: Option<FineTuneCreate>,
    job_id: Option<String>,
    after: Option<String>,
    limit: Option<i64>,
    #[serde(default)]
    args: ClientSettings,
    litellm_params: Option<Map<String, Value>>,
    record: Option<Map<String, Value>>,
    #[serde(default)]
    is_azure: bool,
}

struct Fake {
    is_async: bool,
    reply: Result<Vec<u8>, SdkError>,
    log: Log,
}

impl Fake {
    fn answer(&self, call: Value) -> Result<Vec<u8>, SdkError> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.reply.clone()
    }
}

impl JobsApi for Fake {
    fn is_async(&self) -> bool {
        self.is_async
    }
    fn create(&self, data: &FineTuneCreate) -> Result<Vec<u8>, SdkError> {
        self.answer(json!({ "create": data }))
    }
    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Result<Vec<u8>, SdkError> {
        self.answer(json!({ "list": { "after": after, "limit": limit } }))
    }
    fn retrieve(&self, job_id: &str) -> Result<Vec<u8>, SdkError> {
        self.answer(json!({ "retrieve": { "fine_tuning_job_id": job_id } }))
    }
    fn cancel(&self, job_id: &str) -> Result<Vec<u8>, SdkError> {
        self.answer(json!({ "cancel": { "fine_tuning_job_id": job_id } }))
    }
}

struct Builder {
    mode: String,
    reply: Result<Vec<u8>, SdkError>,
    log: Log,
    constructed: Log,
}

impl ClientBuilder for Builder {
    fn build(
        &self,
        is_async: bool,
        args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        if self.mode == "none" {
            return Ok(None);
        }
        if self.mode == "strict" && args.api_version.is_some() {
            return Err(BuildError::UnexpectedArgument {
                name: "api_version".to_owned(),
            });
        }
        let mut kwargs = json!({
            "api_key": args.api_key.as_ref().map(ExposeSecret::expose_secret),
            "base_url": args.base_url, "timeout": args.timeout, "max_retries": args.max_retries,
            "organization": args.organization, "api_version": args.api_version,
            "litellm_params": args.litellm_params,
        });
        kwargs
            .as_object_mut()
            .map(|map| map.retain(|_, value| !value.is_null()));
        let class = if is_async { "AsyncOpenAI" } else { "OpenAI" };
        self.constructed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({ "class": class, "kwargs": kwargs }));
        Ok(Some(Box::new(Fake {
            is_async,
            reply: self.reply.clone(),
            log: self.log.clone(),
        })))
    }
}

fn base() -> Value {
    json!({
        "id": "ft_1", "created_at": 1, "error": null, "fine_tuned_model": null,
        "finished_at": null, "estimated_finish": null, "integrations": null, "metadata": null,
        "hyperparameters": { "batch_size": null, "learning_rate_multiplier": null, "n_epochs": 3 },
        "method": null, "model": "gpt-4o-mini", "object": "fine_tuning.job",
        "organization_id": "org", "result_files": [], "seed": 5, "status": "queued",
        "trained_tokens": null, "training_file": "file_a", "validation_file": null,
    })
}

fn with(patch: Value) -> Value {
    let mut job = base();
    if let (Some(target), Value::Object(fields)) = (job.as_object_mut(), patch) {
        target.extend(fields);
    }
    job
}

fn bytes_or_error(reply: Option<Reply>) -> Result<Vec<u8>, SdkError> {
    let Some(Reply { status, body }) = reply else {
        return Ok(Vec::new());
    };
    if status < 400 {
        return Ok(body.to_string().into_bytes());
    }
    let text = |name: &str| body["error"][name].as_str().map(str::to_owned);
    Err(SdkError {
        status,
        error_type: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message").unwrap_or_default(),
    })
}

fn outcome(result: Result<Value, FineTuneError>) -> Value {
    match result {
        Ok(value) => json!({ "returned": value }),
        Err(FineTuneError::Sdk { source }) => json!({ "raised": {
            "class": "APIStatusError", "status": source.status, "type": source.error_type,
            "param": source.param, "code": source.code, "message": source.message,
        } }),
        Err(FineTuneError::Build { .. }) => json!({ "raised": { "class": "TypeError" } }),
        Err(FineTuneError::Decode { .. }) => json!({ "raised": { "class": "ValidationError" } }),
        Err(other) => json!({ "raised": { "class": "ValueError", "message": other.to_string() } }),
    }
}

fn run(step: Value) -> Value {
    let step: Step = serde_json::from_value(step).expect("a well-formed step");
    let (log, constructed) = (Log::default(), Log::default());
    let reply = bytes_or_error(step.reply);
    let client = step.client.map(|kind| -> Box<dyn JobsApi> {
        Box::new(Fake {
            is_async: kind.contains("async"),
            reply: reply.clone(),
            log: log.clone(),
        })
    });
    let builder = Builder {
        mode: step.builder.unwrap_or_default(),
        reply,
        log: log.clone(),
        constructed: constructed.clone(),
    };
    let connection = Connection {
        settings: step.args,
        litellm_params: step.litellm_params,
        client,
        is_async: step.is_async,
        builder: &builder,
    };
    let job_id = step.job_id.unwrap_or_default();
    let result = match step.action.as_str() {
        "normalize" => Ok(Value::Object(normalize_fine_tuning_job_dict(
            step.record.unwrap_or_default(),
            step.is_azure,
        ))),
        "project" => connection.get_openai_client().map(|_| {
            constructed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .first()
                .cloned()
                .unwrap_or(json!({ "supplied": true }))
        }),
        "create" => {
            create_fine_tuning_job(connection, &step.data.unwrap_or_default()).map(|job| json!(job))
        }
        "list" => list_fine_tuning_jobs(connection, step.after.as_deref(), step.limit)
            .map(|page| serde_json::from_slice(&page).unwrap_or_default()),
        "retrieve" => retrieve_fine_tuning_job(connection, &job_id).map(|job| json!(job)),
        _ => cancel_fine_tuning_job(connection, &job_id).map(|job| json!(job)),
    };
    let calls = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    json!({ "outcome": outcome(result), "calls": calls })
}

fn returned(value: Value, calls: Value) -> Value {
    json!({ "outcome": { "returned": value }, "calls": calls })
}

fn raised(error: Value) -> Value {
    json!({ "outcome": { "raised": error }, "calls": [] })
}

const STATUSES: [(&str, &str); 8] = [
    ("pending", "queued"),
    ("notRunning", "queued"),
    ("running", "running"),
    ("succeeded", "succeeded"),
    ("failed", "failed"),
    ("canceled", "cancelled"),
    ("canceling", "cancelled"),
    ("paused", "paused"),
];

fn statuses() -> (Value, Value) {
    let steps = STATUSES.map(|(azure, _)| {
        json!({ "action": "normalize", "is_azure": true,
            "record": { "organization_id": "o", "result_files": [], "status": azure } })
    });
    let results = STATUSES.map(|(_, openai)| {
        returned(
            json!({ "organization_id": "o", "result_files": [], "status": openai }),
            json!([]),
        )
    });
    (json!(steps), json!(results))
}

const NOT_ASYNC: &str =
    "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client.";

#[rstest]
#[case::create_method_hyperparameters(
    json!({ "action": "create", "client": "sync", "reply": { "status": 200, "body": with(json!({
        "hyperparameters": { "n_epochs": 3, "extra_hp": "x" }, "extra_top": { "a": 1 },
        "method": { "type": "supervised", "supervised": { "hyperparameters": { "n_epochs": 2 } } },
    })) }, "data": { "model": "gpt-4o-mini", "training_file": "file_a", "suffix": null,
        "extra_body": { "trainingType": 1 },
        "method": { "type": "supervised", "supervised": { "hyperparameters": { "n_epochs": 2 } } },
    } }),
    returned(with(json!({
        "hyperparameters": { "batch_size": null, "learning_rate_multiplier": null, "n_epochs": 3, "extra_hp": "x" },
        "extra_top": { "a": 1 },
        "method": { "type": "supervised", "dpo": null, "reinforcement": null, "supervised": { "hyperparameters": {
            "batch_size": null, "learning_rate_multiplier": null, "n_epochs": 2 } } },
    })), json!([{ "create": { "model": "gpt-4o-mini", "training_file": "file_a", "extra_body": { "trainingType": 1 },
        "method": { "type": "supervised", "supervised": { "hyperparameters": { "n_epochs": 2 } } } } }])),
)]
#[case::list_pagination(
    json!({ "action": "list", "client": "sync", "after": null, "limit": null, "reply": { "status": 200,
        "body": { "object": "list", "data": [with(json!({}))], "has_more": false } } }),
    returned(json!({ "object": "list", "data": [base()], "has_more": false }),
        json!([{ "list": { "after": null, "limit": null } }])),
)]
#[case::retrieve_checkpoint_fields(
    json!({ "action": "retrieve", "client": "sync", "job_id": "ft_1", "reply": { "status": 200, "body": with(json!({
        "status": "succeeded", "fine_tuned_model": "ft:gpt-4o-mini:org::1", "finished_at": 9,
        "result_files": ["file_r"], "trained_tokens": 120, "checkpoint_ids": ["ckpt_1"] })) } }),
    returned(with(json!({ "status": "succeeded", "fine_tuned_model": "ft:gpt-4o-mini:org::1", "finished_at": 9,
        "result_files": ["file_r"], "trained_tokens": 120, "checkpoint_ids": ["ckpt_1"] })),
        json!([{ "retrieve": { "fine_tuning_job_id": "ft_1" } }])),
)]
#[case::cancel_job(
    json!({ "action": "cancel", "client": "sync", "job_id": "ft_1", "reply": { "status": 200,
        "body": with(json!({ "status": "cancelled", "error": { "code": null, "message": null, "param": null } })) } }),
    returned(with(json!({ "status": "cancelled" })), json!([{ "cancel": { "fine_tuning_job_id": "ft_1" } }])),
)]
#[case::azure_null_organization_files(
    json!([
        { "action": "normalize", "is_azure": true, "record": { "organization_id": null, "result_files": null, "status": "queued" } },
        { "action": "normalize", "is_azure": true, "record": { "status": "queued" } },
        { "action": "normalize", "is_azure": true, "record": { "organization_id": "o", "result_files": ["f"] } },
        { "action": "normalize", "is_azure": false, "record": { "organization_id": null, "result_files": null } },
        { "action": "create", "client": "azure_sync", "reply": { "status": 200, "body": with(json!({ "organization_id": null })) },
            "data": { "model": "m", "training_file": "f" } },
    ]),
    json!([
        returned(json!({ "organization_id": "", "result_files": [], "status": "queued" }), json!([])),
        returned(json!({ "organization_id": "", "result_files": [], "status": "queued" }), json!([])),
        returned(json!({ "organization_id": "o", "result_files": ["f"] }), json!([])),
        returned(json!({ "organization_id": null, "result_files": null }), json!([])),
        json!({ "outcome": { "raised": { "class": "ValidationError" } },
            "calls": [{ "create": { "model": "m", "training_file": "f" } }] }),
    ]),
)]
#[case::azure_status_map(statuses().0, statuses().1)]
#[case::sdk_client_argument_projection(
    json!({ "action": "project", "is_async": true, "args": { "api_key": "k", "api_base": "https://x.test", "timeout": 0.0,
        "max_retries": 0, "organization": "" } }),
    returned(json!({ "class": "AsyncOpenAI", "kwargs": { "api_key": "k", "base_url": "https://x.test", "timeout": 0.0,
        "max_retries": 0, "organization": "" } }), json!([])),
)]
#[case::sdk_client_missing(
    json!({ "action": "retrieve", "builder": "none", "job_id": "ft_1" }),
    raised(json!({ "class": "ValueError", "message":
        "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment." })),
)]
#[case::sdk_client_wrong_async(
    json!({ "action": "create", "is_async": true, "client": "sync", "data": { "model": "m", "training_file": "f" } }),
    raised(json!({ "class": "ValueError", "message": NOT_ASYNC })),
)]
#[case::sdk_error_passthrough(
    json!({ "action": "retrieve", "client": "sync", "job_id": "ft_1", "reply": { "status": 404, "body": { "error": {
        "message": "nope", "type": "invalid_request_error", "param": "x", "code": "not_found" } } } }),
    json!({ "outcome": { "raised": { "class": "APIStatusError", "status": 404, "type": "invalid_request_error",
        "param": "x", "code": "not_found", "message": "nope" } }, "calls": [{ "retrieve": { "fine_tuning_job_id": "ft_1" } }] }),
)]
#[case::fine_tune_extra_client_args(
    json!([
        { "action": "project", "builder": "strict", "args": { "api_key": "k", "api_version": "2024-10-21" },
            "litellm_params": { "a": 1 } },
        { "action": "create", "is_async": true, "client": "azure_async", "reply": { "status": 200, "body": base() },
            "data": { "model": "m", "training_file": "f" } },
        { "action": "list", "is_async": true, "client": "azure_async", "reply": { "status": 200, "body": base() } },
        { "action": "retrieve", "is_async": true, "client": "azure_async", "job_id": "ft_1",
            "reply": { "status": 200, "body": base() } },
        { "action": "cancel", "is_async": true, "client": "azure_async", "job_id": "ft_1",
            "reply": { "status": 200, "body": base() } },
    ]),
    json!([
        raised(json!({ "class": "TypeError" })),
        returned(base(), json!([{ "create": { "model": "m", "training_file": "f" } }])),
        returned(base(), json!([{ "list": { "after": null, "limit": null } }])),
        returned(base(), json!([{ "retrieve": { "fine_tuning_job_id": "ft_1" } }])),
        returned(base(), json!([{ "cancel": { "fine_tuning_job_id": "ft_1" } }])),
    ]),
)]
fn fine_tuning_cases(#[case] steps: Value, #[case] expected: Value) {
    let result = match steps {
        Value::Array(items) => Value::Array(items.into_iter().map(run).collect()),
        single => run(single),
    };
    assert_eq!(result, expected);
}

fn settings(key: &str, organization: Option<&str>) -> ClientSettings {
    ClientSettings {
        api_key: Some(SecretString::from(key)),
        api_base: None,
        api_version: None,
        timeout: None,
        max_retries: None,
        organization: organization.map(str::to_owned),
    }
}

fn build_client(builder: &Builder, settings: ClientSettings) -> bool {
    let connection = Connection {
        settings,
        litellm_params: None,
        client: None,
        is_async: false,
        builder,
    };
    connection
        .get_openai_client()
        .is_ok_and(|client| client.is_some())
}

#[test]
fn openai_fine_tuning_credential_identity_isolation() {
    let log = Log::default();
    let builder = Builder {
        mode: String::new(),
        reply: Ok(Vec::new()),
        log: Log::default(),
        constructed: log.clone(),
    };
    assert!(build_client(&builder, settings("sk-alpha", Some("org-a"))));
    assert!(build_client(&builder, settings("sk-beta", Some("org-b"))));
    let built = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        built[0]["kwargs"],
        json!({ "api_key": "sk-alpha", "organization": "org-a" })
    );
    assert_eq!(
        built[1]["kwargs"],
        json!({ "api_key": "sk-beta", "organization": "org-b" })
    );
}

#[test]
fn openai_fine_tuning_secret_safe_failure() {
    let configured = settings("sk-fixture-secret", None);
    let args = ClientArgs {
        api_key: configured.api_key.clone(),
        base_url: None,
        api_version: None,
        timeout: None,
        max_retries: None,
        organization: None,
        litellm_params: json!({ "api_key": "sk-fixture-secret" })
            .as_object()
            .cloned(),
    };
    let missing = Builder {
        mode: "none".to_owned(),
        reply: Ok(Vec::new()),
        log: Log::default(),
        constructed: Log::default(),
    };
    let failure = retrieve_fine_tuning_job(
        Connection {
            settings: settings("sk-fixture-secret", None),
            litellm_params: None,
            client: None,
            is_async: false,
            builder: &missing,
        },
        "ft_1",
    );
    let text = format!(
        "{configured:?} {args:?} {failure:?} {}",
        failure
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default()
    );
    assert!(!text.contains("sk-fixture-secret"), "{text}");
}

#[rstest]
fn replays_the_openai_fine_tuning_corpus(
    #[files("fuzz/corpus/openai_fine_tuning/*")] path: PathBuf,
) {
    let decoded = decode_fine_tune_job(&fs::read(&path).expect("a corpus file"), false);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    assert_eq!(decoded.is_ok(), name.starts_with("valid"), "{name}");
}
