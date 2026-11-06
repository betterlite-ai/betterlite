#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;

use super::client::{
    BuildError, ClientArgs, ClientBuilder, ClientSettings, Connection, JobsApi, SdkError,
};
use super::error::FineTuneError;
use super::job::{FineTuneCreate, decode_fine_tune_job};
use super::normalize::normalize_fine_tuning_job_dict;
use super::{JobRequest, list_fine_tuning_jobs, request_fine_tuning_job};

type Log = Arc<Mutex<Vec<Value>>>;
type Reply = Result<Vec<u8>, SdkError>;

const JOB: &str = r#"{"id":"ft_1","created_at":1,"error":null,"fine_tuned_model":null,"finished_at":null,"estimated_finish":null,"integrations":null,"metadata":null,"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3},"method":null,"model":"gpt-4o-mini","object":"fine_tuning.job","organization_id":"org","result_files":[],"seed":5,"status":"queued","trained_tokens":null,"training_file":"file_a","validation_file":null}"#;
const NOT_ASYNC: &str =
    "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client.";
const MISSING: &str = "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment.";

#[derive(Deserialize)]
struct Planned {
    status: u16,
    body: Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Normalize,
    Project,
    Create,
    List,
    Retrieve,
    Cancel,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BuilderMode {
    #[default]
    Default,
    None,
    Strict,
}

#[derive(Deserialize)]
struct Step {
    action: Action,
    #[serde(default)]
    is_async: bool,
    client: Option<String>,
    #[serde(default)]
    builder: BuilderMode,
    reply: Option<Planned>,
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

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Call<'call> {
    Create(&'call FineTuneCreate),
    List {
        after: Option<&'call str>,
        limit: Option<i64>,
    },
    Retrieve {
        fine_tuning_job_id: &'call str,
    },
    Cancel {
        fine_tuning_job_id: &'call str,
    },
}

#[skip_serializing_none]
#[derive(Serialize)]
struct Kwargs<'call> {
    api_key: Option<&'call str>,
    base_url: Option<&'call str>,
    timeout: Option<f64>,
    max_retries: Option<i64>,
    organization: Option<&'call str>,
    api_version: Option<&'call str>,
    litellm_params: Option<&'call Map<String, Value>>,
}

#[derive(Serialize)]
struct Constructed<'call> {
    class: &'static str,
    kwargs: Kwargs<'call>,
}

#[skip_serializing_none]
#[derive(Serialize)]
struct Raised {
    class: &'static str,
    message: Option<String>,
    status: Option<u16>,
    r#type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Returned(Value),
    Raised(Raised),
}

#[derive(Serialize)]
struct Report {
    outcome: Outcome,
    calls: Vec<Value>,
}

fn record<T>(log: &Log, entry: &T)
where
    T: Serialize,
{
    let value = serde_json::to_value(entry).expect("a serializable log entry");
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(value);
}

struct Fake {
    is_async: bool,
    reply: Reply,
    log: Log,
}

impl Fake {
    fn answer(&self, call: &Call<'_>) -> Reply {
        record(&self.log, call);
        self.reply.clone()
    }
}

impl JobsApi for Fake {
    fn is_async(&self) -> bool {
        self.is_async
    }
    fn create(&self, data: &FineTuneCreate) -> Reply {
        self.answer(&Call::Create(data))
    }
    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Reply {
        self.answer(&Call::List { after, limit })
    }
    fn retrieve(&self, job_id: &str) -> Reply {
        self.answer(&Call::Retrieve {
            fine_tuning_job_id: job_id,
        })
    }
    fn cancel(&self, job_id: &str) -> Reply {
        self.answer(&Call::Cancel {
            fine_tuning_job_id: job_id,
        })
    }
}

struct Builder {
    mode: BuilderMode,
    reply: Reply,
    log: Log,
    constructed: Log,
}

impl ClientBuilder for Builder {
    fn build(
        &self,
        is_async: bool,
        args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        match self.mode {
            BuilderMode::None => return Ok(None),
            BuilderMode::Strict if args.api_version.is_some() => {
                return Err(BuildError::UnexpectedArgument {
                    name: "api_version".to_owned(),
                });
            }
            BuilderMode::Default | BuilderMode::Strict => {}
        }
        let made = Constructed {
            class: if is_async { "AsyncOpenAI" } else { "OpenAI" },
            kwargs: Kwargs {
                api_key: args.api_key.as_ref().map(ExposeSecret::expose_secret),
                base_url: args.base_url.as_deref(),
                timeout: args.timeout,
                max_retries: args.max_retries,
                organization: args.organization.as_deref(),
                api_version: args.api_version.as_deref(),
                litellm_params: args.litellm_params.as_ref(),
            },
        };
        record(&self.constructed, &made);
        Ok(Some(Box::new(Fake {
            is_async,
            reply: self.reply.clone(),
            log: Arc::clone(&self.log),
        })))
    }
}

fn expand(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.into_iter().map(expand).collect()),
        Value::Object(mut fields) => {
            if let Some(Value::Object(changes)) = fields.remove("$job") {
                let mut job: Map<String, Value> = serde_json::from_str(JOB).expect("job JSON");
                job.extend(changes);
                return Value::Object(job);
            }
            Value::Object(
                fields
                    .into_iter()
                    .map(|(key, field)| (key, expand(field)))
                    .collect(),
            )
        }
        scalar @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => scalar,
    }
}

fn parse(text: &str) -> Value {
    expand(serde_json::from_str(text).expect("fixture JSON"))
}

fn planned(reply: Option<&Planned>) -> Reply {
    let Some(Planned { status, body }) = reply else {
        return Ok(Vec::new());
    };
    if *status < 400 {
        return Ok(body.to_string().into_bytes());
    }
    let text = |name: &str| body["error"][name].as_str().map(str::to_owned);
    Err(SdkError {
        status: *status,
        error_type: text("type"),
        param: text("param"),
        code: text("code"),
        message: text("message").unwrap_or_default(),
    })
}

fn raised(error: &FineTuneError) -> Raised {
    let mut report = Raised {
        class: "ValueError",
        message: None,
        status: None,
        r#type: None,
        param: None,
        code: None,
    };
    match error {
        FineTuneError::Sdk { source } => {
            report.class = "APIStatusError";
            report.message = Some(source.message.clone());
            report.status = Some(source.status);
            report.r#type.clone_from(&source.error_type);
            report.param.clone_from(&source.param);
            report.code.clone_from(&source.code);
        }
        FineTuneError::Build { .. } => report.class = "TypeError",
        FineTuneError::Decode { .. } => report.class = "ValidationError",
        FineTuneError::ClientMissing | FineTuneError::NotAsyncClient => {
            report.message = Some(error.to_string());
        }
    }
    report
}

fn dispatch(
    step: &Step,
    connection: Connection<'_>,
    constructed: &Log,
) -> Result<Value, FineTuneError> {
    let job_id = step.job_id.as_deref().unwrap_or_default();
    let job = match step.action {
        Action::Normalize => {
            return Ok(Value::Object(normalize_fine_tuning_job_dict(
                step.record.clone().unwrap_or_default(),
                step.is_azure,
            )));
        }
        Action::Project => {
            return connection.get_openai_client().map(|_| {
                let built = constructed.lock().unwrap_or_else(PoisonError::into_inner);
                built
                    .first()
                    .cloned()
                    .unwrap_or_else(|| parse(r#"{"supplied":true}"#))
            });
        }
        Action::List => {
            let page = list_fine_tuning_jobs(connection, step.after.as_deref(), step.limit)?;
            return serde_json::from_slice(&page)
                .map_err(|source| FineTuneError::Decode { source });
        }
        Action::Create => {
            let data = step.data.clone().unwrap_or_default();
            request_fine_tuning_job(connection, &JobRequest::Create(&data))
        }
        Action::Retrieve => request_fine_tuning_job(connection, &JobRequest::Retrieve(job_id)),
        Action::Cancel => request_fine_tuning_job(connection, &JobRequest::Cancel(job_id)),
    }?;
    serde_json::to_value(job).map_err(|source| FineTuneError::Decode { source })
}

fn run(step: &Step) -> Report {
    let (log, constructed) = (Log::default(), Log::default());
    let reply = planned(step.reply.as_ref());
    let client = step.client.as_ref().map(|kind| -> Box<dyn JobsApi> {
        Box::new(Fake {
            is_async: kind.contains("async"),
            reply: reply.clone(),
            log: Arc::clone(&log),
        })
    });
    let builder = Builder {
        mode: step.builder,
        reply,
        log: Arc::clone(&log),
        constructed: Arc::clone(&constructed),
    };
    let connection = Connection {
        settings: step.args.clone(),
        litellm_params: step.litellm_params.clone(),
        client,
        is_async: step.is_async,
        builder: &builder,
    };
    let outcome = match dispatch(step, connection, &constructed) {
        Ok(value) => Outcome::Returned(value),
        Err(error) => Outcome::Raised(raised(&error)),
    };
    let calls = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Report { outcome, calls }
}

#[rstest]
#[case::create_method_hyperparameters(
    r#"[{"action":"create","client":"sync","reply":{"status":200,"body":{"$job":{"hyperparameters":{"n_epochs":3,"extra_hp":"x"},"extra_top":{"a":1},"method":{"type":"supervised","supervised":{"hyperparameters":{"n_epochs":2}}}}}},"data":{"model":"gpt-4o-mini","training_file":"file_a","suffix":null,"extra_body":{"trainingType":1},"method":{"type":"supervised","supervised":{"hyperparameters":{"n_epochs":2}}}}}]"#,
    r#"[{"outcome":{"returned":{"$job":{"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3,"extra_hp":"x"},"extra_top":{"a":1},"method":{"type":"supervised","dpo":null,"reinforcement":null,"supervised":{"hyperparameters":{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":2}}}}}},"calls":[{"create":{"model":"gpt-4o-mini","training_file":"file_a","extra_body":{"trainingType":1},"method":{"type":"supervised","supervised":{"hyperparameters":{"n_epochs":2}}}}}]}]"#
)]
#[case::list_pagination(
    r#"[{"action":"list","client":"sync","after":null,"limit":null,"reply":{"status":200,"body":{"object":"list","data":[{"$job":{}}],"has_more":false}}}]"#,
    r#"[{"outcome":{"returned":{"object":"list","data":[{"$job":{}}],"has_more":false}},"calls":[{"list":{"after":null,"limit":null}}]}]"#
)]
#[case::retrieve_checkpoint_fields(
    r#"[{"action":"retrieve","client":"sync","job_id":"ft_1","reply":{"status":200,"body":{"$job":{"status":"succeeded","fine_tuned_model":"ft:gpt-4o-mini:org::1","finished_at":9,"result_files":["file_r"],"trained_tokens":120,"checkpoint_ids":["ckpt_1"]}}}}]"#,
    r#"[{"outcome":{"returned":{"$job":{"status":"succeeded","fine_tuned_model":"ft:gpt-4o-mini:org::1","finished_at":9,"result_files":["file_r"],"trained_tokens":120,"checkpoint_ids":["ckpt_1"]}}},"calls":[{"retrieve":{"fine_tuning_job_id":"ft_1"}}]}]"#
)]
#[case::cancel_job(
    r#"[{"action":"cancel","client":"sync","job_id":"ft_1","reply":{"status":200,"body":{"$job":{"status":"cancelled","error":{"code":null,"message":null,"param":null}}}}}]"#,
    r#"[{"outcome":{"returned":{"$job":{"status":"cancelled"}}},"calls":[{"cancel":{"fine_tuning_job_id":"ft_1"}}]}]"#
)]
#[case::azure_null_organization_files(
    r#"[{"action":"normalize","is_azure":true,"record":{"organization_id":null,"result_files":null,"status":"queued"}},{"action":"normalize","is_azure":true,"record":{"status":"queued"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":["f"]}},{"action":"normalize","is_azure":false,"record":{"organization_id":null,"result_files":null}},{"action":"create","client":"azure_sync","reply":{"status":200,"body":{"$job":{"organization_id":null}}},"data":{"model":"m","training_file":"f"}}]"#,
    r#"[{"outcome":{"returned":{"organization_id":"","result_files":[],"status":"queued"}},"calls":[]},{"outcome":{"returned":{"organization_id":"","result_files":[],"status":"queued"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":["f"]}},"calls":[]},{"outcome":{"returned":{"organization_id":null,"result_files":null}},"calls":[]},{"outcome":{"raised":{"class":"ValidationError"}},"calls":[{"create":{"model":"m","training_file":"f"}}]}]"#
)]
#[case::azure_status_map(
    r#"[{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"pending"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"notRunning"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"running"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"succeeded"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"failed"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"canceled"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"canceling"}},{"action":"normalize","is_azure":true,"record":{"organization_id":"o","result_files":[],"status":"paused"}}]"#,
    r#"[{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"queued"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"queued"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"running"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"succeeded"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"failed"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"cancelled"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"cancelled"}},"calls":[]},{"outcome":{"returned":{"organization_id":"o","result_files":[],"status":"paused"}},"calls":[]}]"#
)]
#[case::sdk_client_argument_projection(
    r#"[{"action":"project","is_async":true,"args":{"api_key":"k","api_base":"https://x.test","timeout":0.0,"max_retries":0,"organization":""}}]"#,
    r#"[{"outcome":{"returned":{"class":"AsyncOpenAI","kwargs":{"api_key":"k","base_url":"https://x.test","timeout":0.0,"max_retries":0,"organization":""}}},"calls":[]}]"#
)]
#[case::sdk_client_missing(
    r#"[{"action":"retrieve","builder":"none","job_id":"ft_1"}]"#,
    r#"[{"outcome":{"raised":{"class":"ValueError","message":"MISSING"}},"calls":[]}]"#
)]
#[case::sdk_client_wrong_async(
    r#"[{"action":"create","is_async":true,"client":"sync","data":{"model":"m","training_file":"f"}}]"#,
    r#"[{"outcome":{"raised":{"class":"ValueError","message":"NOT_ASYNC"}},"calls":[]}]"#
)]
#[case::sdk_error_passthrough(
    r#"[{"action":"retrieve","client":"sync","job_id":"ft_1","reply":{"status":404,"body":{"error":{"message":"nope","type":"invalid_request_error","param":"x","code":"not_found"}}}}]"#,
    r#"[{"outcome":{"raised":{"class":"APIStatusError","status":404,"type":"invalid_request_error","param":"x","code":"not_found","message":"nope"}},"calls":[{"retrieve":{"fine_tuning_job_id":"ft_1"}}]}]"#
)]
#[case::fine_tune_extra_client_args(
    r#"[{"action":"project","builder":"strict","args":{"api_key":"k","api_version":"2024-10-21"},"litellm_params":{"a":1}},{"action":"create","is_async":true,"client":"azure_async","reply":{"status":200,"body":{"$job":{}}},"data":{"model":"m","training_file":"f"}},{"action":"list","is_async":true,"client":"azure_async","reply":{"status":200,"body":{"object":"list","data":[{"$job":{}}],"has_more":false}}},{"action":"retrieve","is_async":true,"client":"azure_async","job_id":"ft_1","reply":{"status":200,"body":{"$job":{}}}},{"action":"cancel","is_async":true,"client":"azure_async","job_id":"ft_1","reply":{"status":200,"body":{"$job":{}}}},{"action":"project","args":{"api_key":"k"},"litellm_params":{"a":1}}]"#,
    r#"[{"outcome":{"raised":{"class":"TypeError"}},"calls":[]},{"outcome":{"returned":{"$job":{}}},"calls":[{"create":{"model":"m","training_file":"f"}}]},{"outcome":{"returned":{"object":"list","data":[{"$job":{}}],"has_more":false}},"calls":[{"list":{"after":null,"limit":null}}]},{"outcome":{"returned":{"$job":{}}},"calls":[{"retrieve":{"fine_tuning_job_id":"ft_1"}}]},{"outcome":{"returned":{"$job":{}}},"calls":[{"cancel":{"fine_tuning_job_id":"ft_1"}}]},{"outcome":{"returned":{"class":"OpenAI","kwargs":{"api_key":"k","litellm_params":{"a":1}}}},"calls":[]}]"#
)]
fn fine_tuning_cases(#[case] planned_steps: &str, #[case] expected_text: &str) {
    let steps: Vec<Step> = serde_json::from_value(parse(planned_steps)).expect("well-formed steps");
    let reports: Vec<Report> = steps.iter().map(run).collect();
    let wanted = parse(
        &expected_text
            .replace("NOT_ASYNC", NOT_ASYNC)
            .replace("MISSING", MISSING),
    );
    assert_eq!(serde_json::to_value(&reports).expect("reports"), wanted);
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

fn silent_builder(mode: BuilderMode, constructed: &Log) -> Builder {
    Builder {
        mode,
        reply: Ok(Vec::new()),
        log: Log::default(),
        constructed: Arc::clone(constructed),
    }
}

fn connect(builder: &Builder, settings: ClientSettings) -> Connection<'_> {
    Connection {
        settings,
        litellm_params: None,
        client: None,
        is_async: false,
        builder,
    }
}

#[test]
fn openai_fine_tuning_credential_identity_isolation() {
    let log = Log::default();
    let builder = silent_builder(BuilderMode::Default, &log);
    let alpha = connect(&builder, settings("sk-alpha", Some("org-a"))).get_openai_client();
    let beta = connect(&builder, settings("sk-beta", Some("org-b"))).get_openai_client();
    assert!(alpha.is_ok_and(|built| built.is_some()));
    assert!(beta.is_ok_and(|built| built.is_some()));
    let built = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        built[0]["kwargs"],
        parse(r#"{"api_key":"sk-alpha","organization":"org-a"}"#)
    );
    assert_eq!(
        built[1]["kwargs"],
        parse(r#"{"api_key":"sk-beta","organization":"org-b"}"#)
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
        litellm_params: serde_json::from_str(r#"{"api_key":"sk-fixture-secret"}"#).ok(),
    };
    let missing = silent_builder(BuilderMode::None, &Log::default());
    let request = JobRequest::Retrieve("ft_1");
    let failure = request_fine_tuning_job(connect(&missing, configured.clone()), &request);
    let message = failure.as_ref().err().map(ToString::to_string);
    let rendered = format!("{args:?}");
    assert_eq!(rendered, "ClientArgs { base_url: None, .. }");
    let text = format!("{configured:?} {rendered} {failure:?} {message:?}");
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
