use betterlite_prompts::arguments::json::Object;

// LiteLLM calls response.json() and hands the dict on; a body that is not a JSON object is refused here
pub fn decode_embedding_response(body: &[u8]) -> Result<Object, serde_json::Error> {
    serde_json::from_slice(body)
}
