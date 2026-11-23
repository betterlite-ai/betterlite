use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend::log_payload::cold_storage::ColdStorageHandler;
use betterlite_server::spend::log_payload::content::{
    RequestResponsePayload, resolve_request_response_payload,
};
use betterlite_server::spend::log_payload::daily_summary::{
    DailySummaryRow, SummaryFilters, SummaryValue, daily_summary_items, daily_summary_sql,
};
use betterlite_server::spend::log_payload::metadata::hydrate_log_metadata;
use betterlite_server::spend::log_payload::ports::{
    ActiveLoggers, ColdStorageReader, PayloadLogger,
};
use betterlite_server::spend::visibility::store::PayloadRow;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

// the callback name the Python side configures as litellm.cold_storage_custom_logger
const COLD_LOGGER: &str = "s3_v2";

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Detail { row: DetailRow, cold: ColdAnswer },
    Metadata { values: Vec<JsonData> },
    DailySummary(SummaryStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetailRow {
    request_id: String,
    messages: Option<JsonData>,
    response: Option<JsonData>,
    proxy_server_request: Option<JsonData>,
    metadata: Option<JsonData>,
}

// what the cold-storage logger answers: the stored object, nothing, or a failed read
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColdAnswer {
    payload: Option<Fields>,
    fails: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SummaryStep {
    start_date: String,
    end_date: String,
    api_key: Option<String>,
    request_id: Option<String>,
    user_id: Option<String>,
    rows: Vec<DailySummaryRow>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StepAnswer {
    Detail {
        body: RequestResponsePayload,
        cold_keys: Vec<String>,
    },
    Metadata {
        metadata: Vec<JsonData>,
    },
    DailySummary {
        sql: String,
        params: Vec<String>,
        body: Vec<IndexMap<String, SummaryValue>>,
    },
}

// the configured cold-storage logger, recording every object key it is asked for
struct CaseReader {
    answer: ColdAnswer,
    keys: Mutex<Vec<String>>,
}

#[async_trait]
impl ColdStorageReader for CaseReader {
    async fn proxy_server_request(&self, object_key: &str) -> Result<Option<Fields>, LoggerError> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(object_key.to_owned());
        if self.answer.fails {
            return Err(LoggerError::Delivery {
                callback: COLD_LOGGER.to_owned(),
                message: "scripted cold storage failure".to_owned(),
            });
        }
        Ok(self.answer.payload.clone())
    }
}

// the callbacks LiteLLM's route sees: no payload callbacks, and the one cold-storage logger
struct CaseLoggers {
    reader: Arc<CaseReader>,
}

impl ActiveLoggers for CaseLoggers {
    fn payload_loggers(&self) -> Vec<Arc<dyn PayloadLogger>> {
        Vec::new()
    }

    fn cold_storage_logger(&self, callback_name: &str) -> Option<Arc<dyn ColdStorageReader>> {
        let reader: Arc<dyn ColdStorageReader> = Arc::<CaseReader>::clone(&self.reader);
        (callback_name == COLD_LOGGER).then_some(reader)
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut answers = Vec::new();
        for step in &input.steps {
            answers.push(runtime.block_on(answer(step))?);
        }
        Ok(serde_json::to_value(answers)?)
    }
}

async fn answer(step: &Step) -> Result<StepAnswer, Box<HelperError>> {
    match step {
        Step::Detail { row, cold } => Ok(detail(row, cold).await),
        Step::Metadata { values } => {
            let mut metadata = values.clone();
            for value in &mut metadata {
                hydrate_log_metadata(value);
            }
            Ok(StepAnswer::Metadata { metadata })
        }
        Step::DailySummary(summary) => daily_summary(summary),
    }
}

// the admin's read of one row whose callbacks have nothing, so the row resolves through cold storage
async fn detail(row: &DetailRow, cold: &ColdAnswer) -> StepAnswer {
    let reader = Arc::new(CaseReader {
        answer: cold.clone(),
        keys: Mutex::new(Vec::new()),
    });
    let handler = ColdStorageHandler {
        injected: None,
        selected: Some(COLD_LOGGER.to_owned()),
        loggers: Arc::new(CaseLoggers {
            reader: Arc::clone(&reader),
        }),
    };
    let stored = PayloadRow {
        request_id: row.request_id.clone(),
        messages: row.messages.clone(),
        response: row.response.clone(),
        proxy_server_request: row.proxy_server_request.clone(),
        metadata: row.metadata.clone(),
        user: None,
        team_id: None,
    };
    let body = resolve_request_response_payload(&stored, &handler).await;
    let cold_keys = reader
        .keys
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    StepAnswer::Detail { body, cold_keys }
}

// the legacy route turns each YYYY-MM-DD date into its UTC midnight before building the query
fn daily_summary(summary: &SummaryStep) -> Result<StepAnswer, Box<HelperError>> {
    let query = daily_summary_sql(
        &format!("{}T00:00:00+00:00", summary.start_date),
        &format!("{}T00:00:00+00:00", summary.end_date),
        SummaryFilters {
            api_key: summary.api_key.as_deref(),
            request_id: summary.request_id.as_deref(),
            user_id: summary.user_id.as_deref(),
        },
    );
    let body = daily_summary_items(&summary.rows).or_value_error()?;
    Ok(StepAnswer::DailySummary {
        sql: query.sql,
        params: query.params,
        body,
    })
}
