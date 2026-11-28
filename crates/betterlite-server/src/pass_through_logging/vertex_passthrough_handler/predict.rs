use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::{VertexPassthroughError, VertexPassthroughInput};
use crate::pass_through_logging::vertex_predict_logging::{
    VertexEmbedContentInput, VertexPredictInput, VertexPredictLogger,
};

// LiteLLM calls one of B3639's two reply handlers with almost the same fields; the kind picks which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReplyKind {
    Embed,
    Predict,
}

pub(super) fn reply(
    kind: ReplyKind,
    logger: &VertexPredictLogger,
    input: VertexPassthroughInput<'_>,
) -> Result<HandlerOutcome, VertexPassthroughError> {
    match kind {
        ReplyKind::Embed => {
            let body = input.request_body.map(request_json).transpose()?;
            logger
                .handle_embed_content_response(VertexEmbedContentInput {
                    logging_obj: input.logging_obj,
                    url_route: input.url_route,
                    status: input.status,
                    response_headers: input.response_headers,
                    response_body: input.response_body,
                    request_body: body.as_ref(),
                    kwargs: input.kwargs,
                })
                .map_err(VertexPassthroughError::from)
        }
        // The predict reply carries its own inputs and never reads the caller's request body.
        ReplyKind::Predict => logger
            .handle_predict_response(VertexPredictInput {
                logging_obj: input.logging_obj,
                url_route: input.url_route,
                status: input.status,
                response_headers: input.response_headers,
                response_body: input.response_body,
                kwargs: input.kwargs,
            })
            .map_err(VertexPassthroughError::from),
    }
}

fn request_json(body: &JsonMap) -> Result<Json, VertexPassthroughError> {
    let bytes = serde_json::to_vec(body)
        .map_err(|source| VertexPassthroughError::RequestBody { source })?;
    serde_json::from_slice(&bytes).map_err(|source| VertexPassthroughError::RequestBody { source })
}
