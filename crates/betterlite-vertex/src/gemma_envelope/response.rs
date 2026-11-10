use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;

use super::container::parse_gemma_container_error;
use super::error::GemmaEnvelopeError;

// B010's single-response iterator arrives through this factory, so this leaf never depends on betterlite-provider
pub trait SingleResponseStreamFactory {
    type Stream;

    fn wrap_single_response(&self, response: &ChatResponse) -> Self::Stream;
}

pub enum GemmaReply<Stream> {
    Response(Box<ChatResponse>),
    Stream(Stream),
}

pub fn unwrap_predictions(response: &Json) -> Result<Json, GemmaEnvelopeError> {
    let Json::Object(fields) = response else {
        return Err(GemmaEnvelopeError::MissingPredictions);
    };
    let Some(predictions) = fields.get("predictions") else {
        return Err(GemmaEnvelopeError::MissingPredictions);
    };
    if let Some(error) = parse_gemma_container_error(predictions) {
        return Err(GemmaEnvelopeError::Container {
            code: error.code,
            message: error.message,
        });
    }
    Ok(predictions.clone())
}

pub fn handle_fake_stream_response<Factory>(
    response: ChatResponse,
    stream_requested: bool,
    factory: &Factory,
) -> GemmaReply<Factory::Stream>
where
    Factory: SingleResponseStreamFactory,
{
    if stream_requested {
        GemmaReply::Stream(factory.wrap_single_response(&response))
    } else {
        GemmaReply::Response(Box::new(response))
    }
}
