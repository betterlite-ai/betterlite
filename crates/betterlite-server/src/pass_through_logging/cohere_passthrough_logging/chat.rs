use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::cohere_chat_v2::reply::{
    AssistantReply, CohereV2Reply, ReplyInput, decode_cohere_chat_v2,
};
use indexmap::IndexMap;
use uuid::Uuid;

use super::super::base_passthrough_logging::{
    PassthroughLoggingError, PassthroughProvider, UpstreamChatReply,
};
use super::stream;
use super::{converted, unix_seconds};

// Cohere's passthrough provider: the V2 chat config for every route and the V1 collected stream builder.
pub struct CohereProvider;

impl PassthroughProvider for CohereProvider {
    fn provider_name(&self) -> ProviderId {
        ProviderId::Cohere
    }

    // CohereV2ChatConfig.transform_response reads only the body and sets created, model and usage.
    fn transform_chat_response(
        &self,
        model: &str,
        upstream: &UpstreamChatReply<'_>,
    ) -> Result<Json, PassthroughLoggingError> {
        let reply = decode_cohere_chat_v2(ReplyInput {
            body: upstream.body,
            status: upstream.status_code,
            model,
            created: unix_seconds(),
            id: format!("chatcmpl-{}", Uuid::new_v4()),
            finish_reason: None,
            message: AssistantReply::default(),
            next_id: || Uuid::new_v4().to_string(),
        })
        .map_err(PassthroughLoggingError::transform)?;
        logged_model_response(&reply)
    }

    fn build_complete_streaming_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughLoggingError> {
        stream::complete_streaming_response(all_chunks, model)
    }
}

// LiteLLM's ModelResponse() defaults plus what transform_response fills, as model_dump(exclude_unset) shows it.
fn logged_model_response(reply: &CohereV2Reply) -> Result<Json, PassthroughLoggingError> {
    let usage = converted::<_, Json>(&reply.usage).map_err(PassthroughLoggingError::transform)?;
    let mut fields = IndexMap::new();
    fields.insert("id".to_owned(), Json::Text(reply.id.clone()));
    fields.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    fields.insert(
        "created".to_owned(),
        Json::Number(serde_json::Number::from(reply.created)),
    );
    fields.insert("model".to_owned(), Json::Text(reply.model.clone()));
    fields.insert("system_fingerprint".to_owned(), Json::Null);
    fields.insert(
        "choices".to_owned(),
        Json::List(vec![logged_choice(reply)?]),
    );
    fields.insert("usage".to_owned(), usage);
    Ok(Json::Object(fields))
}

// The default Choices carries finish_reason "stop": transform_response never sets one.
fn logged_choice(reply: &CohereV2Reply) -> Result<Json, PassthroughLoggingError> {
    let reason = reply
        .finish_reason
        .clone()
        .unwrap_or_else(|| "stop".to_owned());
    let mut fields = IndexMap::new();
    fields.insert("finish_reason".to_owned(), Json::Text(reason));
    fields.insert(
        "index".to_owned(),
        Json::Number(serde_json::Number::from(0_u64)),
    );
    fields.insert("message".to_owned(), logged_message(reply)?);
    Ok(Json::Object(fields))
}

// Message() sets content, role, function_call and tool_calls; annotations only appear when set.
fn logged_message(reply: &CohereV2Reply) -> Result<Json, PassthroughLoggingError> {
    let message = &reply.message;
    let content = match &message.content {
        Some(text) => Json::Text(text.clone()),
        None => Json::Null,
    };
    let mut fields = IndexMap::new();
    fields.insert("content".to_owned(), content);
    fields.insert("role".to_owned(), Json::Text("assistant".to_owned()));
    fields.insert("function_call".to_owned(), Json::Null);
    fields.insert("tool_calls".to_owned(), logged_tool_calls(message)?);
    // Message() carries annotations only when the transform set them, an empty list included.
    if let Some(annotations) = &message.annotations {
        let logged = converted::<_, Json>(annotations.as_slice())
            .map_err(PassthroughLoggingError::transform)?;
        fields.insert("annotations".to_owned(), logged);
    }
    Ok(Json::Object(fields))
}

// Message() turns an empty tool_calls list into null; the decoder already reports it that way.
fn logged_tool_calls(message: &AssistantReply) -> Result<Json, PassthroughLoggingError> {
    match &message.tool_calls {
        Some(calls) => {
            converted::<_, Json>(calls.as_slice()).map_err(PassthroughLoggingError::transform)
        }
        None => Ok(Json::Null),
    }
}
