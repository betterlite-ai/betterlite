mod normalize;
mod tests;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_logging::datadog_logging::error::DatadogError;
use betterlite_logging::datadog_logging::logger::{DatadogDeps, DatadogLogger, uuid_id};
use betterlite_logging::datadog_logging::payloads::{DatadogLoggingKwargs, SyncCall};
use betterlite_logging::datadog_logging::settings::{DatadogInit, DatadogInitParams};
use betterlite_logging::datadog_logging::transport::{
    PostRequest, Transport, TransportFailure, TransportResponse,
};
use betterlite_logging::datadog_logging::{batch, health, inspection, sync};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Scenario {
    name: String,
    environment: BTreeMap<String, String>,
    init: Init,
    replies: Vec<Reply>,
    actions: Vec<Action>,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Init {
    dd_api_key: Option<String>,
    dd_site: Option<String>,
    dd_agent_host: Option<String>,
    dd_agent_port: Option<String>,
    allow_env_credentials: Option<bool>,
    turn_off_message_logging: Option<bool>,
    use_v1: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
struct Reply {
    status: u16,
    body: String,
    fail: bool,
    #[serde(rename = "raise_status")]
    _raise_status: Option<bool>,
}

#[derive(Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
enum Action {
    Success { payload: Value },
    Failure { payload: Value },
    Flush,
    SendBatch,
    HealthCheck,
    SyncLog { payload: Value },
    GetPayload,
}

impl Action {
    const fn name(&self) -> &'static str {
        match self {
            Self::Success { .. } => "success",
            Self::Failure { .. } => "failure",
            Self::Flush => "flush",
            Self::SendBatch => "send_batch",
            Self::HealthCheck => "health_check",
            Self::SyncLog { .. } => "sync_log",
            Self::GetPayload => "get_payload",
        }
    }
}

#[derive(Debug, Default)]
struct TransportState {
    replies: VecDeque<Reply>,
    calls: Vec<Value>,
    action: String,
}

struct ScriptedTransport {
    state: Mutex<TransportState>,
}

impl ScriptedTransport {
    fn set_action(&self, action: &str) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.action != action {
            action.clone_into(&mut state.action);
        }
    }

    fn new(replies: &[Reply]) -> Self {
        Self {
            state: Mutex::new(TransportState {
                replies: replies.iter().map(Reply::clone).collect(),
                calls: Vec::new(),
                action: String::new(),
            }),
        }
    }
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        let (action, reply) = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let reply = state.replies.pop_front().unwrap_or_default();
            (state.action.clone(), reply)
        };
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let call = json!({
            "action": action,
            "url": request.url,
            "headers": headers,
            "body": normalize::body(&request.body, &request.headers, &action),
        });
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .push(call);
        if reply.fail {
            return Err(TransportFailure::Request(TransportError::Rejected(
                HttpError::Timeout,
            )));
        }
        Ok(TransportResponse {
            status: reply.status,
            body: reply.body.into_bytes(),
        })
    }
}

struct FixedClock(Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}

fn value_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn moment() -> DateTime<FixedOffset> {
    DateTime::from_timestamp(1_704_067_200, 0).map_or_else(
        || DateTime::UNIX_EPOCH.fixed_offset(),
        |moment| moment.fixed_offset(),
    )
}

fn event(payload: StandardLoggingPayload) -> Arc<LoggerEvent> {
    Arc::new(LoggerEvent {
        payload: Arc::new(payload),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        stream: false,
        guardrail_snapshot: None,
    })
}

fn build_init(init: &Init) -> DatadogInit {
    DatadogInit {
        dd_api_key: init.dd_api_key.clone().map(SecretString::from),
        dd_site: init.dd_site.clone(),
        dd_agent_host: init.dd_agent_host.clone(),
        dd_agent_port: init.dd_agent_port.clone(),
        allow_env_credentials: init.allow_env_credentials.unwrap_or(true),
        params: DatadogInitParams {
            turn_off_message_logging: init.turn_off_message_logging.unwrap_or(false),
        },
        use_v1: init.use_v1.unwrap_or(false),
    }
}

const fn classify(error: &DatadogError) -> &'static str {
    match error {
        DatadogError::MissingApiKey => "missing_api_key",
        DatadogError::MissingSite => "missing_site",
        DatadogError::Environment(_)
        | DatadogError::Payload(_)
        | DatadogError::Json(_)
        | DatadogError::ApiKeyHeader(_)
        | DatadogError::Compress(_)
        | DatadogError::MissingStandardLoggingObject
        | DatadogError::MissingLegacyKwargs => "invalid_configuration",
    }
}

async fn perform(
    logger: &Arc<DatadogLogger>,
    transport: &Arc<ScriptedTransport>,
    action: &Action,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    transport.set_action(action.name());
    match action {
        Action::Success { payload } => log_async(logger, payload, false).await,
        Action::Failure { payload } => log_async(logger, payload, true).await,
        Action::Flush => {
            batch::flush_queue(logger).await;
            Ok(json!({"queue_length": logger.queue.pending().len()}))
        }
        Action::SendBatch => {
            let _report = batch::async_send_batch(logger).await;
            Ok(json!({"queue_length": logger.queue.pending().len()}))
        }
        Action::HealthCheck => {
            let status = health::async_health_check(logger).await;
            Ok(json!({"health": serde_json::to_value(status).map_err(value_error)?}))
        }
        Action::SyncLog { payload } => sync_log(logger, payload).await,
        Action::GetPayload => {
            let found = inspection::get_request_response_payload(logger, "request-id", None, None);
            Ok(json!({"payload": found.and_then(|fields| serde_json::to_value(fields).ok())}))
        }
    }
}

async fn log_async(
    logger: &Arc<DatadogLogger>,
    payload: &Value,
    failure: bool,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let event = event(serde_json::from_value(payload.clone()).map_err(value_error)?);
    if failure {
        logger
            .async_log_failure_event(event)
            .await
            .map_err(value_error)?;
    } else {
        logger
            .async_log_success_event(event)
            .await
            .map_err(value_error)?;
    }
    Ok(json!({"queue_length": logger.queue.pending().len()}))
}

async fn sync_log(
    logger: &Arc<DatadogLogger>,
    payload: &Value,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let kwargs = DatadogLoggingKwargs {
        standard_logging_object: Some(
            serde_json::from_value(payload.clone()).map_err(value_error)?,
        ),
        legacy: None,
    };
    let call = SyncCall {
        kwargs: &kwargs,
        response_obj: &Json::Null,
        start_time: moment(),
        end_time: moment(),
    };
    sync::log_success_event(logger, &call).await;
    Ok(json!({"queue_length": logger.queue.pending().len()}))
}

fn run_scenario(scenario: &Scenario) -> Result<Value, Box<crate::helpers::HelperError>> {
    let init = build_init(&scenario.init);
    let transport = Arc::new(ScriptedTransport::new(&scenario.replies));
    let cloned = Arc::clone(&transport);
    let concrete: Arc<dyn Transport> = cloned;
    let logger = match DatadogLogger::new(
        &init,
        &scenario.environment,
        DatadogDeps {
            transport: concrete,
            clock: Arc::new(FixedClock(Duration::from_secs(1))),
            ids: uuid_id,
            tasks: None,
        },
    ) {
        Ok(logger) => logger,
        Err(error) => {
            return Ok(json!({
                "name": scenario.name,
                "init": {"error": classify(&error)},
            }));
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(value_error)?;
    let results = runtime.block_on(async {
        let mut results = Vec::new();
        for action in &scenario.actions {
            results.push(perform(&logger, &transport, action).await?);
        }
        Ok::<Vec<Value>, Box<crate::helpers::HelperError>>(results)
    })?;
    Ok(scenario_report(
        &scenario.name,
        &logger,
        &transport,
        &results,
    ))
}

fn scenario_report(
    name: &str,
    logger: &DatadogLogger,
    transport: &ScriptedTransport,
    results: &[Value],
) -> Value {
    json!({
        "name": name,
        "init": {
            "batch_size": logger.settings.batch.batch_size,
            "intake_url": logger.settings.intake_url,
            "authenticated": logger.settings.api_key.is_some(),
            "turn_off_message_logging": logger.settings.options.turn_off_message_logging,
            "error": Value::Null,
        },
        "results": results,
        "calls": transport
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .calls
            .clone(),
        "queue": queue_report(logger),
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut scenarios = Vec::new();
        for scenario in &input.scenarios {
            scenarios.push(run_scenario(scenario)?);
        }
        Ok(json!({"scenarios": scenarios}))
    }
}
