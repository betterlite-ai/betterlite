use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_openai::api::image_contracts::records::{ImageHttpResponse, ImageResponse};
use indexmap::IndexMap;

use super::error::ImageEditError;
use super::execute::response_text;
use super::plan::{EditCall, EditPlan, PreparedRequest};
use crate::canvas_edits::selection::{ImageEditConfig, image_edit_config_for_model};
use crate::canvas_results;
use crate::image_call::plan::LogContext;
use crate::image_call::transport::ImageReply;
use crate::stability_edits;

#[derive(Debug)]
pub struct EditReply {
    pub response: ImageResponse,
    pub additional_headers: AdditionalHeaders,
}

// What a successful transport observed, so the family transform reads the original reply bytes.
pub struct ObservedReply<'reply> {
    pub prepared: &'reply PreparedRequest,
    pub reply: &'reply ImageReply,
    pub now_seconds: i64,
}

// Every family decodes the raw body again; the handler's own parse decides only the null and syntax errors.
pub fn transform_response<Lookup>(
    call: &EditCall<'_>,
    plan: &EditPlan<'_, '_>,
    observed: &ObservedReply<'_>,
    lookup: &Lookup,
    log: &mut LogContext<'_, '_>,
) -> Result<EditReply, ImageEditError>
where
    Lookup: canvas_results::ModelPriceLookup + stability_edits::reply::ModelPriceLookup,
{
    log_post_call(call, observed, log);
    check_outer_reply(&observed.reply.body)?;
    let upstream = ImageHttpResponse {
        status_code: observed.reply.status,
        headers: observed.reply.headers.iter().cloned().collect(),
        body: observed.reply.body.clone(),
    };
    let config = image_edit_config_for_model(
        call.model,
        plan.model_cost,
        plan.resolution,
        &mut ModelCostKeys::default(),
    )?;
    match config {
        ImageEditConfig::Stability => {
            let answer = stability_edits::reply::transform_image_edit_response(
                call.model,
                &upstream,
                observed.now_seconds,
                lookup,
            )?;
            Ok(EditReply {
                response: answer.response,
                additional_headers: answer.additional_headers,
            })
        }
        ImageEditConfig::NovaCanvas => {
            let answer = canvas_results::transform_image_edit_response(
                call.model,
                &upstream,
                observed.now_seconds,
                lookup,
            )?;
            Ok(EditReply {
                response: answer.response,
                additional_headers: answer.additional_headers,
            })
        }
    }
}

// response.json() rejects a body that is not JSON and stops at a None; a parsed value of another shape goes on.
fn check_outer_reply(body: &[u8]) -> Result<(), ImageEditError> {
    match serde_json::from_slice::<Json>(body)? {
        Json::Null => Err(ImageEditError::NullReply),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => Ok(()),
    }
}

fn log_post_call(call: &EditCall<'_>, observed: &ObservedReply<'_>, log: &mut LogContext<'_, '_>) {
    let text = response_text(&observed.reply.body);
    let input = match call.prompt {
        Some(prompt) => Json::Text(prompt.to_owned()),
        None => Json::Null,
    };
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(text.clone()),
            input,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(observed.prepared.data.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    tracing::debug!("raw model_response: {text}");
}
