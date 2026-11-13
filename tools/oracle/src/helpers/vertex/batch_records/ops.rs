use betterlite_models::decisions::Json;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use betterlite_vertex::gemini_usage::usage::calculate_usage;
use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_batch_records::create::to_batch_job;
use betterlite_vertex::vertex_batch_records::response::{
    BatchList, to_batch_list, to_batch_object,
};
use betterlite_vertex::vertex_batch_records::rows::{
    NativeVertexBatchRowStats, RowCostCalculator, VERTEX_AI_PROVIDER, native_vertex_batch_row_stats,
};
use betterlite_vertex::vertex_batch_records::types::{
    VertexBatchJobPage, VertexBatchPredictionResponse,
};
use jiff::tz::TimeZone;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Create { calls: Vec<CreateCall> },
    Job { zone: String, calls: Vec<Value> },
    List { zone: String, calls: Vec<Value> },
    Row { calls: Vec<RowCall> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCall {
    data: Value,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowCall {
    row: Json,
    model_name: Option<String>,
    model_info: Option<ModelInfo>,
    pricing: Pricing,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Pricing {
    Fixed,
    RequiresInfo,
    Failure,
}

struct FixturePricing(Pricing);

impl RowCostCalculator for FixturePricing {
    fn calculate(
        &self,
        _usage: &Usage,
        _model: &str,
        custom_llm_provider: &str,
        model_info: Option<&ModelInfo>,
    ) -> Result<GenericCostResult, TokenCostError> {
        let missing_info = matches!(self.0, Pricing::RequiresInfo) && model_info.is_none();
        let failure = matches!(self.0, Pricing::Failure) || missing_info;
        if failure || custom_llm_provider != VERTEX_AI_PROVIDER {
            return Err(TokenCostError::UnknownModel);
        }
        Ok(GenericCostResult {
            prompt_cost: Decimal::new(25, 2),
            completion_cost: Decimal::new(5, 1),
        })
    }
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    match input {
        Input::Create { calls } => {
            let outcomes = calls
                .iter()
                .map(create_outcome)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"calls": outcomes}))
        }
        Input::Job { zone, calls } => {
            let local = local_zone(zone)?;
            let outcomes: Vec<Value> = calls.iter().map(|job| job_outcome(job, &local)).collect();
            Ok(json!({"calls": outcomes}))
        }
        Input::List { zone, calls } => {
            let local = local_zone(zone)?;
            let outcomes: Vec<Value> = calls
                .iter()
                .map(|page| list_outcome(page, &local))
                .collect();
            Ok(json!({"calls": outcomes}))
        }
        Input::Row { calls } => Ok(json!({
            "calls": calls.iter().map(row_outcome).collect::<Vec<_>>(),
        })),
    }
}

fn local_zone(zone: &str) -> Result<LocalTimeContext, Box<HelperError>> {
    match TimeZone::get(zone) {
        Ok(time_zone) => Ok(LocalTimeContext { zone: time_zone }),
        Err(error) => Err(invalid_input(format!("unknown time zone {zone}: {error}"))),
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn create_outcome(call: &CreateCall) -> Result<Value, Box<HelperError>> {
    let request =
        serde_json::from_value::<CreateBatchRequest>(call.data.clone()).or_value_error()?;
    let job = to_batch_job(
        &request,
        call.vertex_project.as_deref(),
        call.vertex_location.as_deref(),
        Uuid::nil(),
    );
    match job {
        Ok(created) => Ok(json!({
            "ok": true,
            "job": serde_json::to_value(&created).or_value_error()?,
        })),
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn job_outcome(response: &Value, local: &LocalTimeContext) -> Value {
    match batch_of(response, local) {
        Ok(batch) => json!({"ok": true, "batch": job_state(&batch)}),
        Err(status) => json!({"ok": false, "status": status}),
    }
}

// a payload that is not a batch response has no provider status, and a projection error keeps its own
fn batch_of(response: &Value, local: &LocalTimeContext) -> Result<BatchObject, Option<u16>> {
    let job = serde_json::from_value::<VertexBatchPredictionResponse>(response.clone())
        .map_err(|_| None)?;
    to_batch_object(&job, local).map_err(|error| error.status_code())
}

fn list_outcome(page: &Value, local: &LocalTimeContext) -> Value {
    let Ok(decoded) = serde_json::from_value::<VertexBatchJobPage>(page.clone()) else {
        return json!({"ok": false, "status": Value::Null});
    };
    match to_batch_list(&decoded, local) {
        Ok(list) => json!({"ok": true, "page": page_state(&list)}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn page_state(list: &BatchList) -> Value {
    json!({
        "object": "list",
        "data": list.data.iter().map(job_state).collect::<Vec<_>>(),
        "first_id": list.first_id,
        "last_id": list.last_id,
        "has_more": list.has_more,
        "next_page_token": list.next_page_token,
    })
}

fn row_outcome(call: &RowCall) -> Value {
    let stats = native_vertex_batch_row_stats(
        &call.row,
        call.model_name.as_deref(),
        call.model_info.as_ref(),
        calculate_usage,
        &FixturePricing(call.pricing),
    );
    match stats {
        Ok(Some(priced)) => json!({"ok": true, "stats": stats_state(&priced)}),
        Ok(None) => json!({"ok": true, "stats": Value::Null}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn job_state(batch: &BatchObject) -> Value {
    json!({
        "id": &batch.id,
        "completion_window": &batch.completion_window,
        "created_at": batch.created_at,
        "endpoint": &batch.endpoint,
        "input_file_id": &batch.input_file_id,
        "object": batch.object,
        "status": batch.status,
        "error_file_id": &batch.error_file_id,
        "output_file_id": &batch.output_file_id,
    })
}

fn stats_state(stats: &NativeVertexBatchRowStats) -> Value {
    json!({
        "usage": usage_state(&stats.usage),
        "total_tokens": stats.total_tokens,
        "model": stats.model.as_deref(),
        "prompt_cost": stats.prompt_cost.normalize().to_string(),
        "completion_cost": stats.completion_cost.normalize().to_string(),
    })
}

fn usage_state(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
}
