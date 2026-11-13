use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CountCallError;

pub type CountResponse = IndexMap<String, Json>;

pub fn decode_bedrock_count_reply(bytes: &[u8]) -> Result<CountResponse, CountCallError> {
    match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Object(fields)) => Ok(fields),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CountCallError::NotAnObject)
        }
        Err(source) => Err(CountCallError::Decode(source)),
    }
}
