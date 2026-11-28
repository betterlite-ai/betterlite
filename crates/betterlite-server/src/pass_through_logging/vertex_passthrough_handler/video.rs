use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_openai::videos::reply::VideoObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::studio_veo_operation::records::{VideoContext, create_response};
use betterlite_vertex::vertex_relay_metadata::url_models::extract_model_from_url;
use rust_decimal::Decimal;

use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::gemini_passthrough_logging::cost::{
    CostRequest, CostResponse, PassthroughCompletionCost,
};

pub(super) fn video(
    cost: &dyn PassthroughCompletionCost,
    input: VertexPassthroughInput<'_>,
    vertex_location: Option<&str>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    let model = extract_model_from_url(input.url_route);
    let request_data = input.request_body.map(request_object).transpose()?;
    let video = build_video(input.response_body, request_data.as_ref(), &model)?;
    record_model(input.logging_obj, &model);
    let response_cost = video_cost(cost, &video, &model, vertex_location)?;
    record_cost(input.logging_obj, response_cost);
    Ok(HandlerOutcome {
        result: Some(PassThroughResponse::VideoOperation(video_body(&video)?)),
        kwargs: priced_kwargs(input.kwargs, response_cost, model),
    })
}

fn build_video(
    body: &[u8],
    request_data: Option<&Object>,
    model: &str,
) -> Result<VideoObject, VertexPassthroughError> {
    let settings = VertexSettings::default();
    let context = VideoContext {
        provider: Some("vertex_ai"),
        request_data,
        settings: &settings,
    };
    create_response(model, body, &context).map_err(VertexPassthroughError::from)
}

fn record_model(logging_obj: &StreamLoggingObject, model: &str) {
    logging_obj.edit(&mut |state| {
        model.clone_into(&mut state.model);
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text(model.to_owned()));
        state.model_call_details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text("vertex_ai".to_owned()),
        );
        state.custom_llm_provider = Some("vertex_ai".to_owned());
    });
}

fn video_cost(
    cost: &dyn PassthroughCompletionCost,
    video: &VideoObject,
    model: &str,
    vertex_location: Option<&str>,
) -> Result<Decimal, VertexPassthroughError> {
    cost.completion_cost(&CostRequest {
        response: CostResponse::Video(video),
        model,
        custom_llm_provider: "vertex_ai",
        vertex_location,
        call_type: CallType::CreateVideo,
    })
    .map_err(|source| VertexPassthroughError::Cost {
        model: model.to_owned(),
        source,
    })
}

fn record_cost(logging_obj: &StreamLoggingObject, response_cost: Decimal) {
    logging_obj.edit(&mut |state| {
        state
            .model_call_details
            .insert("response_cost".to_owned(), decimal_json(response_cost));
    });
}

fn priced_kwargs(
    kwargs: PassThroughLoggingKwargs,
    response_cost: Decimal,
    model: String,
) -> PassThroughLoggingKwargs {
    let mut entries = kwargs.entries;
    entries.insert("response_cost".to_owned(), decimal_json(response_cost));
    entries.insert("model".to_owned(), Json::Text(model));
    entries.insert(
        "custom_llm_provider".to_owned(),
        Json::Text("vertex_ai".to_owned()),
    );
    PassThroughLoggingKwargs { entries }
}

fn request_object(body: &JsonMap) -> Result<Object, VertexPassthroughError> {
    let bytes = serde_json::to_vec(body)
        .map_err(|source| VertexPassthroughError::RequestBody { source })?;
    serde_json::from_slice(&bytes).map_err(|source| VertexPassthroughError::RequestBody { source })
}

// LiteLLM's model_dump excludes the video object's private hidden params, so the logged body is its fields.
fn video_body(video: &VideoObject) -> Result<Json, VertexPassthroughError> {
    serde_json::to_value(video)
        .and_then(serde_json::from_value)
        .map_err(|source| VertexPassthroughError::Encode { source })
}
