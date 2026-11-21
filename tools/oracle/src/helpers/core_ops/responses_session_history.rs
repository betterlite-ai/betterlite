use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::api::responses_session_history::loads_session_lookup_settings;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::bfl::poll::PollSleeper;
use betterlite_core::api::responses_session_history::error::SessionHistoryError;
use betterlite_core::api::responses_session_history::merge::{
    CompletionRequest, MergedSession, merge_session,
};
use betterlite_core::api::responses_session_history::{
    ColdStorageReader, SessionHistory, SessionLog, SpendLogQuery,
};
use betterlite_core::api::responses_tool_repair::value::Value as Message;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::Level;
use tracing::subscriber::with_default;

const MODULE: &str = "betterlite_core::api::responses_session_history";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    previous_response_id: String,
    database: bool,
    disable_spend_logs: bool,
    replies: Vec<Vec<SessionLog>>,
    cold_storage: bool,
    cold_objects: IndexMap<String, Option<IndexMap<String, Json>>>,
    cold_failures: Vec<String>,
    request: CompletionRequest,
}

#[derive(Debug, thiserror::Error)]
#[error("cold storage could not read {object_key}")]
pub(crate) struct ColdStorageFailure {
    object_key: String,
}

#[derive(Default)]
struct Effects {
    queries: Vec<Value>,
    reads: Vec<String>,
    sleeps: Vec<f64>,
}

// one script answers the database, the bucket and the clock, and keeps what each was asked
struct Script<'input> {
    input: &'input Input,
    effects: Mutex<Effects>,
}

impl SpendLogQuery for Script<'_> {
    async fn session_logs(
        &self,
        sql: &'static str,
        request_id: &str,
    ) -> Result<Vec<SessionLog>, TableError> {
        // LiteLLM indents its query over several lines; both sides record it with single spaces
        let flat_sql = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        effects
            .queries
            .push(json!({"sql": flat_sql, "request_id": request_id}));
        let position = effects.queries.len().saturating_sub(1);
        Ok(self
            .input
            .replies
            .get(position)
            .cloned()
            .unwrap_or_default())
    }
}

impl ColdStorageReader for Script<'_> {
    type Error = ColdStorageFailure;

    async fn proxy_server_request(
        &self,
        object_key: &str,
    ) -> Result<Option<IndexMap<String, Json>>, ColdStorageFailure> {
        let mut effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        effects.reads.push(object_key.to_owned());
        if self
            .input
            .cold_failures
            .iter()
            .any(|failing| failing == object_key)
        {
            return Err(ColdStorageFailure {
                object_key: object_key.to_owned(),
            });
        }
        Ok(self.input.cold_objects.get(object_key).cloned().flatten())
    }
}

impl PollSleeper for Script<'_> {
    async fn sleep(&self, duration: Duration) {
        let mut effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        effects.sleeps.push(duration.as_secs_f64());
    }
}

type Service<'deps> = SessionHistory<'deps, Script<'deps>, Script<'deps>, Script<'deps>>;
type Outcome = Result<MergedSession, SessionHistoryError<ColdStorageFailure>>;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
        let diagnostics = Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        };
        let cache = DualLocalCache::new(
            None,
            None,
            &Settings::default(),
            Arc::new(|| 0.0_f64),
            diagnostics,
        );
        let tool_calls = IndexMap::<String, Message>::new();
        let script = Script {
            input,
            effects: Mutex::new(Effects::default()),
        };
        let service = SessionHistory {
            repository: input.database.then_some(&script),
            cold_storage: input.cold_storage.then_some(&script),
            sleeper: &script,
            lookup: loads_session_lookup_settings(&BTreeMap::<String, String>::new())
                .or_config_error()?,
            disable_spend_logs: input.disable_spend_logs,
            limits: ApiLimits::default(),
            input_cache: &cache,
            tool_calls: &tool_calls,
        };
        let (outcome, log) = traced(&service, input)?;
        answer(outcome, &script, &log)
    }
}

fn traced(service: &Service<'_>, input: &Input) -> Result<(Outcome, String), Box<HelperError>> {
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    let written = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::clone(&written))
        .without_time()
        .finish();
    let outcome = with_default(subscriber, || {
        runtime.block_on(merge_session(
            service,
            &input.previous_response_id,
            &input.request,
        ))
    });
    let bytes = written
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok((outcome, String::from_utf8(bytes).or_value_error()?))
}

// dispatch.py reports a raised exception by its status alone, with type, param and code null
fn answer(outcome: Outcome, script: &Script<'_>, log: &str) -> HelperResult {
    let merged = match outcome {
        Ok(merged) => merged,
        Err(SessionHistoryError::NoMessages(class)) => {
            return Err(Box::new(HelperError {
                class: "BadRequestError".to_owned(),
                status: Some(class.metadata().status_code),
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: "unable to create messages".to_owned(),
            }));
        }
        Err(other) => return Err(other).or_value_error(),
    };
    let effects = script
        .effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "queries": effects.queries,
        "reads": effects.reads,
        "sleeps": effects.sleeps,
        "debug": debug_records(log)?,
        "messages": merged.messages.into_iter().map(attributes).collect::<Vec<_>>(),
        "litellm_trace_id": merged.litellm_trace_id,
    }))
}

#[derive(Deserialize)]
struct Record {
    target: String,
    fields: RecordFields,
}

#[derive(Deserialize)]
struct RecordFields {
    #[serde(default)]
    message: String,
}

// a message's later lines carry the JSON dump LiteLLM formats its own way, so only the first is compared
fn debug_records(log: &str) -> Result<Vec<String>, Box<HelperError>> {
    let mut records = Vec::new();
    for line in log.lines() {
        let record = serde_json::from_str::<Record>(line).or_value_error()?;
        if record.target.starts_with(MODULE) {
            let first = record.fields.message.lines().next().unwrap_or_default();
            records.push(first.to_owned());
        }
    }
    Ok(records)
}

// a replayed Message is an attribute object in LiteLLM; its model_dump is what the request carries
fn attributes(message: Message) -> Message {
    if let Message::Instance { object } = message {
        Message::Dict(object.attrs)
    } else {
        message
    }
}
