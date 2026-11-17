use betterlite_logging::langfuse_projections::steering::truthiness;
use betterlite_models::decisions::{Extra, Json};

const PREDICTIONS: &str = "predictions";
const AUDIO_CONTENT: &str = "audioContent";
const ENCODED_AUDIO: &str = "bytesBase64Encoded";
const MULTIMODAL_KEYS: [&str; 3] = ["textEmbedding", "imageEmbedding", "videoEmbeddings"];

// LiteLLM counts the prediction entries holding an audio payload; the truthiness is Python's bool().
pub(super) fn audio_prediction_count(body: &Extra) -> usize {
    let Some(Json::List(predictions)) = body.get(PREDICTIONS) else {
        return 0;
    };
    let mut count = 0_usize;
    for prediction in predictions {
        if counts_as_audio(prediction) {
            count = count.saturating_add(1);
        }
    }
    count
}

// The detection is key presence, not truthiness, so a null textEmbedding still names a multimodal reply.
pub(super) fn is_multimodal_embedding(body: &Extra) -> bool {
    let Some(Json::List(predictions)) = body.get(PREDICTIONS) else {
        return false;
    };
    predictions.iter().any(has_embedding_key)
}

fn counts_as_audio(prediction: &Json) -> bool {
    let Json::Object(fields) = prediction else {
        return false;
    };
    let audio = fields.get(AUDIO_CONTENT).is_some_and(truthiness);
    let encoded = fields.get(ENCODED_AUDIO).is_some_and(truthiness);
    audio || encoded
}

fn has_embedding_key(prediction: &Json) -> bool {
    let Json::Object(fields) = prediction else {
        return false;
    };
    MULTIMODAL_KEYS.iter().any(|key| fields.contains_key(*key))
}
