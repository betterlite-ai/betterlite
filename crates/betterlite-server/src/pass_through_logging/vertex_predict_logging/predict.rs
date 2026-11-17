use std::collections::BTreeMap;

use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_vertex::endpoint_target::passthrough::get_vertex_location_from_url;
use betterlite_vertex::model_policy::lyria::LyriaOverlay;
use betterlite_vertex::multimodal_embedding_records::response as multimodal;
use betterlite_vertex::vertex_image_policy::prediction::{
    is_image_generation_response,
    // ast-grep-ignore: no-long-names
    process_image_generation_response,
};
use betterlite_vertex::vertex_imagen_generation::response::ImagenGenerationResponse;
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse, CustomLlmProvider,
};
use betterlite_vertex::vertex_relay_metadata::url_models::extract_model_from_url;
use betterlite_vertex::vertex_text_embeddings::response as text_embedding;
use rust_decimal::Decimal;

use super::cost;
use super::detect::{audio_prediction_count, is_multimodal_embedding};
use super::spend;
use super::wire::{json_of, object_fields};
use super::{VertexPredictError, VertexPredictInput};

const IMAGE_CALL_TYPE: &str = "passthrough-image-generation";

struct Prediction {
    result: PassThroughResponse,
    cost_response: CompletionResponse,
}

pub(super) fn predict(
    input: &VertexPredictInput<'_>,
    cost_service: &dyn CompletionCost,
    catalog: &ModelCatalog,
) -> Result<HandlerOutcome, VertexPredictError> {
    let body: Json = serde_json::from_slice(input.response_body)
        .map_err(|source| VertexPredictError::Decode { source })?;
    let fields = object_fields(&body)?;
    let model = extract_model_from_url(input.url_route);
    if let Some(response_cost) = audio_cost(fields, &model, catalog)? {
        let result = PassThroughResponse::Standard(PassThroughResponseObject { response: body });
        return Ok(priced_outcome(input, model, response_cost, result));
    }
    let prediction = prediction(input, &body, fields, &model)?;
    let response_cost = cost_service
        .completion_cost(&CompletionCostRequest {
            response: &prediction.cost_response,
            model: &model,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: get_vertex_location_from_url(input.url_route),
        })
        .ok_or_else(|| VertexPredictError::Cost {
            model: model.clone(),
        })?;
    Ok(priced_outcome(
        input,
        model,
        response_cost,
        prediction.result,
    ))
}

// Both priced paths write the same three keys and the same logging fields, with the literal vertex_ai provider.
fn priced_outcome(
    input: &VertexPredictInput<'_>,
    model: String,
    response_cost: Decimal,
    result: PassThroughResponse,
) -> HandlerOutcome {
    let spend = MediaSpend {
        model,
        custom_llm_provider: CustomLlmProvider::VertexAi.as_str(),
        response_cost: Some(response_cost),
    };
    spend::outcome(input.logging_obj, &spend, result, &input.kwargs)
}

// The audio price is the Lyria catalog price times the counted predictions; the cost trait is never called.
fn audio_cost(
    fields: &Extra,
    model: &str,
    catalog: &ModelCatalog,
) -> Result<Option<Decimal>, VertexPredictError> {
    let count = audio_prediction_count(fields);
    if count == 0 {
        return Ok(None);
    }
    let runtime = BTreeMap::new();
    let overlay = LyriaOverlay {
        runtime: &runtime,
        bundled: catalog,
    };
    let Some(price) = overlay.generation_cost(model) else {
        return Ok(None);
    };
    let prediction_count = i64::try_from(count).map_err(|_error| VertexPredictError::Cost {
        model: model.to_owned(),
    })?;
    let total = price
        .checked_mul(Decimal::from(prediction_count))
        .ok_or_else(|| VertexPredictError::Cost {
            model: model.to_owned(),
        })?;
    Ok(Some(total))
}

fn prediction(
    input: &VertexPredictInput<'_>,
    body: &Json,
    fields: &Extra,
    model: &str,
) -> Result<Prediction, VertexPredictError> {
    if is_image_generation_response(fields)? {
        return image(input, fields);
    }
    if is_multimodal_embedding(fields) {
        return multimodal(input, model);
    }
    let mut response = text_embedding::transform_response(body, model)?;
    embedding_prediction(&mut response, model)
}

fn image(input: &VertexPredictInput<'_>, fields: &Extra) -> Result<Prediction, VertexPredictError> {
    let mut response = ImagenGenerationResponse::default();
    // ast-grep-ignore: no-long-names
    process_image_generation_response(fields, &mut response)?;
    input.logging_obj.edit(&mut |state| {
        IMAGE_CALL_TYPE.clone_into(&mut state.call_type);
    });
    Ok(Prediction {
        result: PassThroughResponse::Image(json_of(&response)?),
        cost_response: cost::image_response(),
    })
}

// LiteLLM hands the multimodal step an empty request, so this reply fails on the missing instances.
fn multimodal(
    input: &VertexPredictInput<'_>,
    model: &str,
) -> Result<Prediction, VertexPredictError> {
    let request = Json::Object(Extra::new());
    let answer = multimodal::transform_response(
        input.status.as_u16(),
        input.response_body,
        &request,
        model,
    )?;
    let mut response = answer.response;
    embedding_prediction(&mut response, model)
}

// Only an embedding result gets the URL model put back; the image result carries no model at all.
fn embedding_prediction(
    response: &mut EmbeddingResponse,
    model: &str,
) -> Result<Prediction, VertexPredictError> {
    model.clone_into(&mut response.model);
    Ok(Prediction {
        result: PassThroughResponse::Embedding(json_of(response)?),
        cost_response: cost::embedding_response(
            model,
            response.usage.prompt_tokens,
            response.usage.total_tokens,
        ),
    })
}
