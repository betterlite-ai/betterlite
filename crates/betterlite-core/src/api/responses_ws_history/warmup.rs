use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::response_id_codecs::response::decode_previous_response_id;

use super::error::ResponsesWsHistoryError;
use super::serialization::Chunk;
use super::values::has_content;
use super::{FrameSink, ManagedResponsesWebSocket};

pub const WARMUP_RESPONSE_ID_PREFIX: &str = "resp_warmup_";

// _is_warmup_frame: the nested response wins when it is a nonempty object, and generate must be exactly false
#[must_use]
pub fn is_warmup_frame(message: &IndexMap<String, Json>) -> bool {
    let source = match message.get("response") {
        Some(Json::Object(nested)) if !nested.is_empty() => nested,
        _ => message,
    };
    matches!(source.get("generate"), Some(Json::Bool(false)))
}

// _warmup_source_params: the nested response, or the flat event without its type
#[must_use]
pub fn warmup_source_params(message: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    match message.get("response") {
        Some(Json::Object(nested)) if !nested.is_empty() => nested.clone(),
        _ => {
            let mut source = message.clone();
            let _removed = source.shift_remove("type");
            source
        }
    }
}

// _is_warmup_response_id: only a synthetic id this connection invented skips the history lookup
#[must_use]
pub fn is_warmup_response_id(response_id: Option<&str>) -> bool {
    let Some(response_id) = response_id.filter(|value| !value.is_empty()) else {
        return false;
    };
    decode_previous_response_id(response_id).starts_with(WARMUP_RESPONSE_ID_PREFIX)
}

impl<Sink: FrameSink> ManagedResponsesWebSocket<Sink> {
    // _build_warmup_response: a completed Responses object that never reaches a provider
    #[must_use]
    pub fn build_warmup_response(
        &self,
        message: &IndexMap<String, Json>,
    ) -> IndexMap<String, Json> {
        let source = warmup_source_params(message);
        let wire_model = match source.get("model").filter(|value| has_content(value)) {
            Some(model) => model.clone(),
            None => match self.model_group.as_ref().filter(|group| !group.is_empty()) {
                Some(group) => Json::Text(group.clone()),
                None => Json::Text(self.model.clone()),
            },
        };
        let mut usage = IndexMap::new();
        usage.insert("input_tokens".to_owned(), Json::Number(Number::from(0_u8)));
        usage.insert("output_tokens".to_owned(), Json::Number(Number::from(0_u8)));
        usage.insert("total_tokens".to_owned(), Json::Number(Number::from(0_u8)));
        let mut response = IndexMap::new();
        response.insert(
            "id".to_owned(),
            Json::Text(format!("{WARMUP_RESPONSE_ID_PREFIX}{}", (self.ids)())),
        );
        response.insert("object".to_owned(), Json::Text("response".to_owned()));
        response.insert(
            "created_at".to_owned(),
            Json::Number(Number::from((self.clock)())),
        );
        response.insert("status".to_owned(), Json::Text("completed".to_owned()));
        response.insert("model".to_owned(), wire_model);
        response.insert("output".to_owned(), Json::List(Vec::new()));
        response.insert("usage".to_owned(), Json::Object(usage));
        response
    }

    // _send_warmup_ack: response.created then response.completed, dropping only a frame that fails to serialize
    pub async fn send_warmup_ack(
        &self,
        message: &IndexMap<String, Json>,
    ) -> Result<(), ResponsesWsHistoryError> {
        let response = self.build_warmup_response(message);
        for (event_type, status) in [
            ("response.created", "in_progress"),
            ("response.completed", "completed"),
        ] {
            let mut seen = response.clone();
            seen.insert("status".to_owned(), Json::Text(status.to_owned()));
            let mut event = IndexMap::new();
            event.insert("type".to_owned(), Json::Text(event_type.to_owned()));
            event.insert("response".to_owned(), Json::Object(seen));
            let data = Json::Object(event);
            let Some(serialized) = (self.serializer)(Chunk::Json(&data)) else {
                continue;
            };
            self.websocket.send_text(&serialized).await?;
        }
        Ok(())
    }
}
