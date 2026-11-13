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
use betterlite_vertex::vertex_batch_records::response::{to_batch_list, to_batch_object};
use betterlite_vertex::vertex_batch_records::rows::{
    NativeVertexBatchRowStats, RowCostCalculator, RowCostRequest, VERTEX_AI_PROVIDER,
    native_vertex_batch_row_stats,
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
    Job { zone: String, calls: Vec<JobCall> },
    List { zone: String, calls: Vec<ListCall> },
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
pub(crate) struct JobCall {
    response: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListCall {
    page: Value,
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

struct FixturePricing {
    pricing: Pricing,
}

impl RowCostCalculator for FixturePricing {
    fn calculate(&self, request: &RowCostRequest<'_>) -> Result<GenericCostResult, TokenCostError> {
        let failure = matches!(self.pricing, Pricing::Failure);
        let missing_info =
            matches!(self.pricing, Pricing::RequiresInfo) && request.model_info.is_none();
        let wrong_provider = request.custom_llm_provider != VERTEX_AI_PROVIDER;
        if failure || missing_info || wrong_provider {
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
            let local = local_context(zone)?;
            let outcomes: Vec<Value> = calls.iter().map(|call| job_outcome(call, &local)).collect();
            Ok(json!({"calls": outcomes}))
        }
        Input::List { zone, calls } => {
            let local = local_context(zone)?;
            let outcomes: Vec<Value> = calls
                .iter()
                .map(|call| list_outcome(call, &local))
                .collect();
            Ok(json!({"calls": outcomes}))
        }
        Input::Row { calls } => {
            let outcomes: Vec<Value> = calls.iter().map(row_outcome).collect();
            Ok(json!({"calls": outcomes}))
        }
    }
}

fn create_outcome(call: &CreateCall) -> Result<Value, Box<HelperError>> {
    let request =
        serde_json::from_value::<CreateBatchRequest>(call.data.clone()).or_value_error()?;
    // the display name carries a fresh uuid, so the oracle cases compare every other field
    let job = to_batch_job(
        &request,
        call.vertex_project.as_deref(),
        call.vertex_location.as_deref(),
        Uuid::nil(),
    );
    match job {
        Ok(created) => {
            let value = serde_json::to_value(&created).or_value_error()?;
            Ok(json!({"ok": true, "job": value}))
        }
        Err(error) => Ok(json!({"ok": false, "status": error.status_code()})),
    }
}

fn job_outcome(call: &JobCall, local: &LocalTimeContext) -> Value {
    let Ok(response) =
        serde_json::from_value::<VertexBatchPredictionResponse>(call.response.clone())
    else {
        return json!({"ok": false, "status": Value::Null});
    };
    match to_batch_object(&response, local) {
        Ok(batch) => json!({"ok": true, "batch": job_state(&batch)}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn list_outcome(call: &ListCall, local: &LocalTimeContext) -> Value {
    let Ok(page) = serde_json::from_value::<VertexBatchJobPage>(call.page.clone()) else {
        return json!({"ok": false, "status": Value::Null});
    };
    match to_batch_list(&page, local) {
        Ok(list) => json!({
            "ok": true,
            "page": {
                "object": "list",
                "data": list.data.iter().map(job_state).collect::<Vec<_>>(),
                "first_id": list.first_id,
                "last_id": list.last_id,
                "has_more": list.has_more,
                "next_page_token": list.next_page_token,
            },
        }),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn row_outcome(call: &RowCall) -> Value {
    let stats = native_vertex_batch_row_stats(
        &call.row,
        call.model_name.as_deref(),
        call.model_info.as_ref(),
        calculate_usage,
        &FixturePricing {
            pricing: call.pricing,
        },
    );
    match stats {
        Ok(Some(priced)) => json!({"ok": true, "stats": stats_state(&priced)}),
        Ok(None) => json!({"ok": true, "stats": Value::Null}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn local_context(zone: &str) -> Result<LocalTimeContext, Box<HelperError>> {
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
    let prompt = usage
        .prompt_tokens_details
        .as_ref()
        .map_or(Value::Null, |details| {
            json!({
                "text_tokens": details.text_tokens,
                "audio_tokens": details.audio_tokens,
                "image_tokens": details.image_tokens,
                "video_tokens": details.video_tokens,
                "cached_tokens": details.cached_tokens,
                "tool_use_tokens": details.tool_use_tokens,
                "web_search_requests": details.web_search_requests,
                "google_maps_grounding_requests": details.google_maps_grounding_requests,
            })
        });
    let completion = usage
        .completion_tokens_details
        .as_ref()
        .map_or(Value::Null, |details| {
            json!({
                "text_tokens": details.text_tokens,
                "audio_tokens": details.audio_tokens,
                "image_tokens": details.image_tokens,
                "video_tokens": details.video_tokens,
                "reasoning_tokens": details.reasoning_tokens,
            })
        });
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt,
        "completion_tokens_details": completion,
    })
}
