use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::fields::response::InitResponseObject;
use crate::fields::timestamps::{Moment, TimestampError, cleanup_timestamps};
use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::{ResponseUsage, Usage, get_usage_as_dict};
use crate::logging_metadata::params::MetadataMap;
use crate::logging_privacy::internal::{Origin, call_category, is_unbilled};
use crate::payload::compose::{PayloadComposition, PayloadRequest};
use crate::payload::error::PayloadError;
use crate::payload::response::{init_response_object, object_field};

// get_usage_as_dict: the combined usage wins, an unbilled read reports zeros, else the response's usage
pub(crate) fn usage_object(
    request: &PayloadRequest<'_>,
    metadata: &MetadataMap,
    composition: &PayloadComposition<'_>,
) -> Result<Json, PayloadError> {
    let combined = combined_usage(
        request
            .state
            .model_call_details
            .get("combined_usage_object"),
    )?;
    let response = if combined.is_some() || unbilled(request, metadata) {
        ResponseUsage::Absent
    } else {
        response_usage(request.init_response_obj)?
    };
    get_usage_as_dict(
        response,
        combined.as_ref(),
        composition.sources.responses_converter.as_ref(),
    )
    .map_err(PayloadError::from)
}

fn combined_usage(value: Option<&Json>) -> Result<Option<Usage>, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => usage_from_mapping(fields)
            .map(Some)
            .map_err(PayloadError::from),
        Some(_) => Err(PayloadError::FieldType {
            field: "combined_usage_object",
        }),
    }
}

fn response_usage(init: &Json) -> Result<ResponseUsage<'static>, PayloadError> {
    let body = match init_response_object(init) {
        InitResponseObject::Absent | InitResponseObject::Other => IndexMap::new(),
        InitResponseObject::Mapping(fields) => fields,
        InitResponseObject::Projected { body, .. } => body,
    };
    match body.get("usage") {
        None | Some(Json::Null) => Ok(ResponseUsage::Absent),
        Some(Json::Object(fields)) => Ok(ResponseUsage::Mapping(fields.clone())),
        Some(_) => Err(PayloadError::FieldType {
            field: "response.usage",
        }),
    }
}

// is_unbilled_non_inference_call reads the merged metadata, where merge_litellm_metadata leaves the origin
fn unbilled(request: &PayloadRequest<'_>, metadata: &MetadataMap) -> bool {
    let background = matches!(
        object_field(request.init_response_obj, "background"),
        Some(Json::Bool(true))
    );
    let origin = match metadata.get("internal_call_origin") {
        Some(Json::Text(origin)) if origin == "background_response_cost_poll" => {
            Some(Origin::BackgroundResponseCostPoll)
        }
        Some(_) | None => None,
    };
    is_unbilled(
        call_category(Some(request.state.call_type.as_str())),
        origin,
        background,
    )
}

pub(crate) struct Totals {
    pub total: i64,
    pub prompt: i64,
    pub completion: i64,
}

pub(crate) fn totals(usage: &Json) -> Result<Totals, PayloadError> {
    let fields = match usage {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(PayloadError::FieldType { field: "usage" });
        }
    };
    Ok(Totals {
        total: token_count(fields.get("total_tokens"), "total_tokens")?,
        prompt: token_count(fields.get("prompt_tokens"), "prompt_tokens")?,
        completion: token_count(fields.get("completion_tokens"), "completion_tokens")?,
    })
}

fn token_count(value: Option<&Json>, field: &'static str) -> Result<i64, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(PayloadError::FieldType { field }),
        Some(_) => Err(PayloadError::FieldType { field }),
    }
}

pub(crate) struct Timing {
    pub start: Decimal,
    pub end: Decimal,
    pub completion_start: Decimal,
    pub response: Decimal,
}

pub(crate) fn timing(request: &PayloadRequest<'_>) -> Result<Timing, PayloadError> {
    let start = Moment::DateTime(request.start_time);
    let end = Moment::DateTime(request.end_time);
    let completion = request.state.completion_start_time.map(Moment::DateTime);
    let stamps = cleanup_timestamps(Some(&start), Some(&end), completion.as_ref())?;
    let response_end = if request.state.stream == Some(true) {
        stamps.completion_start_time
    } else {
        stamps.end_time
    };
    let response_time = response_end
        .checked_sub(stamps.start_time)
        .ok_or(TimestampError::ResponseTimeOverflow)?;
    Ok(Timing {
        start: stamps.start_time,
        end: stamps.end_time,
        completion_start: stamps.completion_start_time,
        response: response_time,
    })
}
