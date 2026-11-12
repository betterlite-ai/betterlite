use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_logging::s3_batch::Element;
use betterlite_logging::s3_batch::flush::SystemMonotonic;
use betterlite_logging::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use betterlite_logging::s3_transport::batch_element::ContentType;
use betterlite_logging::s3_transport::error::S3TransportError;
use betterlite_logging::s3_transport::signing::Clock;
use betterlite_logging::s3_transport::transport::{S3HttpClient, S3Request, S3Response};
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    source: Source<()>,
    now: u64,
    #[serde(default)]
    cold_storage_logger: Option<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
pub(crate) struct Step {
    #[serde(default)]
    enqueue: Vec<ElementInput>,
    #[serde(default)]
    replies: Vec<ReplyRule>,
}

#[derive(Deserialize)]
pub(crate) struct ElementInput {
    s3_object_key: String,
    s3_object_download_filename: String,
    payload: Json,
    #[serde(default)]
    retrying_since_offset: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ReplyRule {
    #[serde(rename = "match")]
    key_match: String,
    responses: Vec<Reply>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Reply {
    status: u16,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Default)]
struct Script {
    rules: Vec<ReplyRule>,
    served: BTreeMap<String, usize>,
    requests: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct ScriptedS3 {
    script: Mutex<Script>,
}

impl S3HttpClient for ScriptedS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        script.requests.push((
            request.url.clone(),
            String::from_utf8(request.body.clone()).unwrap_or_default(),
        ));
        let matched = script
            .rules
            .iter()
            .find(|rule| request.url.contains(&rule.key_match))
            .cloned();
        let mut reply = Reply {
            status: 200,
            body: String::new(),
        };
        if let Some(rule) = matched {
            let index = script.served.entry(rule.key_match).or_insert(0);
            if let Some(scripted) = rule.responses.get(*index) {
                reply = scripted.clone();
            }
            *index = index.saturating_add(1);
        }
        drop(script);
        Box::pin(async move {
            tokio::task::yield_now().await;
            Ok(S3Response {
                status: reply.status,
                body: reply.body.into_bytes(),
            })
        })
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| fail("RuntimeError", &error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

fn normalise(url: &str) -> String {
    let Some(end) = url.find(".jsonl") else {
        return url.to_owned();
    };
    let start = end.saturating_sub(32);
    match (url.get(..start), url.get(end..)) {
        (Some(head), Some(tail)) => format!("{head}ID{tail}"),
        _ => url.to_owned(),
    }
}

fn fail(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

async fn execute(input: &Input) -> HelperResult {
    let client = Arc::new(ScriptedS3::default());
    let logger = build_logger(input, &client)?;
    let steps = run_steps(&logger, client.as_ref(), input).await;
    drop(logger);
    Ok(json!({"steps": steps}))
}

fn build_logger(
    input: &Input,
    client: &Arc<ScriptedS3>,
) -> Result<Arc<S3BatchLogger>, Box<HelperError>> {
    let environment: Arc<dyn betterlite_config::loader::EnvironmentLookup + Send + Sync> =
        Arc::new(BTreeMap::<String, String>::new());
    let parameters = resolve_s3_parameters::<()>(
        &Source::default(),
        &input.source,
        None,
        environment.as_ref(),
    );
    let http_client: Arc<dyn S3HttpClient> = Arc::<ScriptedS3>::clone(client);
    S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: http_client,
            clock: Arc::new(FixedClock(
                SystemTime::UNIX_EPOCH
                    .checked_add(Duration::from_secs(input.now))
                    .unwrap_or(SystemTime::UNIX_EPOCH),
            )),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment,
            throttle_observer: None,
            cold_storage_logger: input.cold_storage_logger.clone(),
            tasks: None,
            shutdown: CancellationToken::new(),
        },
    )
    .map_err(|error| fail("InitError", &error.to_string()))
}

async fn run_steps(logger: &S3BatchLogger, client: &ScriptedS3, input: &Input) -> Vec<Value> {
    let mut steps = Vec::new();
    for step in &input.steps {
        steps.push(run_step(logger, client, step).await);
    }
    steps
}

async fn run_step(logger: &S3BatchLogger, client: &ScriptedS3, step: &Step) -> Value {
    *client.script.lock().unwrap_or_else(PoisonError::into_inner) = Script {
        rules: step.replies.clone(),
        served: BTreeMap::new(),
        requests: Vec::new(),
    };
    for element in &step.enqueue {
        logger.queue.enqueue(element_for(element));
    }
    if let Err(error) = logger.flush_queue().await {
        tracing::debug!(%error, "the oracle step kept records for the next flush");
    }
    step_value(logger, client)
}

fn element_for(element: &ElementInput) -> Element {
    Element {
        payload: element.payload.clone(),
        s3_object_key: element.s3_object_key.clone(),
        s3_object_download_filename: element.s3_object_download_filename.clone(),
        body: None,
        content_type: ContentType::Json,
        retrying_since: element.retrying_since_offset,
    }
}

fn step_value(logger: &S3BatchLogger, client: &ScriptedS3) -> Value {
    let pending = logger.queue.snapshot();
    let mut requests = client
        .script
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .requests
        .clone();
    requests.sort();
    json!({
        "requests": requests
            .iter()
            .map(|(url, body)| json!({"url": normalise(url), "body": body}))
            .collect::<Vec<_>>(),
        "queue": pending
            .entries
            .iter()
            .map(|element| json!({
                "key": element.s3_object_key,
                "retrying_since": element.retrying_since.is_some(),
                "body": element.body,
            }))
            .collect::<Vec<Value>>(),
        "requeued_count": pending.requeued_count,
        "limit": logger.uploads.snapshot().limit,
    })
}
