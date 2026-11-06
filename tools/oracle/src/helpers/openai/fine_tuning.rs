use std::sync::{Arc, Mutex, PoisonError};

use betterlite_openai::fine_tuning::client::{
    BuildError, ClientArgs, ClientBuilder, ClientSettings, Connection, JobsApi, SdkError,
};
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use betterlite_openai::fine_tuning::normalize::normalize_fine_tuning_job_dict;
use betterlite_openai::fine_tuning::{JobRequest, list_fine_tuning_jobs, request_fine_tuning_job};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::HelperResult;

mod tests;

pub(crate) struct Op;

type Log = Arc<Mutex<Vec<Value>>>;
type Reply = Result<Vec<u8>, SdkError>;

#[derive(Clone, Deserialize)]
struct Planned {
    status: u16,
    body: Value,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Clone, Copy, Deserialize)]
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

#[derive(Clone, Deserialize)]
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

struct Recorded {
    is_async: bool,
    reply: Reply,
    log: Log,
}

impl Recorded {
    fn answer(&self, call: Value) -> Reply {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.reply.clone()
    }
}

impl JobsApi for Recorded {
    fn is_async(&self) -> bool {
        self.is_async
    }
    fn create(&self, data: &FineTuneCreate) -> Reply {
        self.answer(json!({ "create": data }))
    }
    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Reply {
        self.answer(json!({ "list": { "after": after, "limit": limit } }))
    }
    fn retrieve(&self, job_id: &str) -> Reply {
        self.answer(json!({ "retrieve": { "fine_tuning_job_id": job_id } }))
    }
    fn cancel(&self, job_id: &str) -> Reply {
        self.answer(json!({ "cancel": { "fine_tuning_job_id": job_id } }))
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
        let mut kwargs = json!({
            "api_key": args.api_key.as_ref().map(ExposeSecret::expose_secret),
            "base_url": args.base_url, "timeout": args.timeout, "max_retries": args.max_retries,
            "organization": args.organization, "api_version": args.api_version,
            "litellm_params": args.litellm_params,
        });
        if let Some(map) = kwargs.as_object_mut() {
            map.retain(|_, value| !value.is_null());
        }
        let class = if is_async { "AsyncOpenAI" } else { "OpenAI" };
        self.constructed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({ "class": class, "kwargs": kwargs }));
        Ok(Some(Box::new(Recorded {
            is_async,
            reply: self.reply.clone(),
            log: Arc::clone(&self.log),
        })))
    }
}

fn planned_reply(planned: Option<Planned>) -> Reply {
    let Some(Planned { status, body }) = planned else {
        return Ok(b"{}".to_vec());
    };
    if status < 400 {
        return Ok(body.to_string().into_bytes());
    }
    let text = |name: &str| {
        let field = body.get("error").and_then(|error| error.get(name));
        field.and_then(Value::as_str).map(str::to_owned)
    };
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
        Err(error @ (FineTuneError::ClientMissing | FineTuneError::NotAsyncClient)) => {
            json!({ "raised": { "class": "ValueError", "message": error.to_string() } })
        }
    }
}

fn perform(
    step: Step,
    connection: Connection<'_>,
    constructed: &Log,
) -> Result<Value, FineTuneError> {
    let job_id = step.job_id.unwrap_or_default();
    match step.action {
        Action::Normalize => Ok(Value::Object(normalize_fine_tuning_job_dict(
            step.record.unwrap_or_default(),
            step.is_azure,
        ))),
        Action::Project => connection.get_openai_client().map(|_| {
            let built = constructed.lock().unwrap_or_else(PoisonError::into_inner);
            built
                .first()
                .cloned()
                .unwrap_or_else(|| json!({ "supplied": true }))
        }),
        Action::Create => {
            let data = step.data.unwrap_or_default();
            request_fine_tuning_job(connection, &JobRequest::Create(&data)).map(|job| json!(job))
        }
        Action::List => {
            let page = list_fine_tuning_jobs(connection, step.after.as_deref(), step.limit)?;
            serde_json::from_slice(&page).map_err(|source| FineTuneError::Decode { source })
        }
        Action::Retrieve => request_fine_tuning_job(connection, &JobRequest::Retrieve(&job_id))
            .map(|job| json!(job)),
        Action::Cancel => {
            request_fine_tuning_job(connection, &JobRequest::Cancel(&job_id)).map(|job| json!(job))
        }
    }
}

fn run_step(mut step: Step) -> Value {
    let (log, constructed) = (Log::default(), Log::default());
    let reply = planned_reply(step.reply.take());
    let client = step.client.take().map(|kind| -> Box<dyn JobsApi> {
        Box::new(Recorded {
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
        settings: std::mem::take(&mut step.args),
        litellm_params: step.litellm_params.take(),
        client,
        is_async: step.is_async,
        builder: &builder,
    };
    let result = perform(step, connection, &constructed);
    let calls = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
    json!({ "outcome": outcome(result), "calls": calls })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(Value::Array(
            input.steps.iter().cloned().map(run_step).collect(),
        ))
    }
}
