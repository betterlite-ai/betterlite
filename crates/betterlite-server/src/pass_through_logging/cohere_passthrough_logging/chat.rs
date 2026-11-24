use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::cohere_chat_v2::reply::citations::Annotation;
use betterlite_openai::cohere_chat_v2::reply::tool_calls::ReplyToolCall;
use betterlite_openai::cohere_chat_v2::reply::{
    AssistantReply, CohereV2Reply, ReplyInput, decode_cohere_chat_v2,
};
use betterlite_openai::cohere_streams::CohereUsage;
use serde::Serialize;
use uuid::Uuid;

use super::super::base_passthrough_logging::{
    PassthroughLoggingError, PassthroughProvider, PassthroughStage, UpstreamChatReply,
};
use super::stream;
use super::unix_seconds;

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
        .map_err(|source| PassthroughStage::Transform.failed(source))?;
        logged_model_response(&reply)
    }

    // ast-grep-ignore: no-long-names - B3637's OpenAI provider implements this fixed name
    fn build_complete_streaming_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, PassthroughLoggingError> {
        stream::complete_streaming_response(all_chunks, model)
    }
}

// LiteLLM's ModelResponse() defaults plus what transform_response fills, as model_dump(exclude_unset) shows it.
#[derive(Serialize)]
struct LoggedResponse<'reply> {
    id: &'reply str,
    object: &'static str,
    created: u64,
    model: &'reply str,
    system_fingerprint: Option<&'reply str>,
    choices: [LoggedChoice<'reply>; 1],
    usage: &'reply CohereUsage,
}

#[derive(Serialize)]
struct LoggedChoice<'reply> {
    finish_reason: &'reply str,
    index: u32,
    message: LoggedMessage<'reply>,
}

// Message() sets content, role, function_call and tool_calls; annotations only appear when set, an empty list included.
#[derive(Serialize)]
struct LoggedMessage<'reply> {
    content: Option<&'reply str>,
    role: &'static str,
    function_call: Option<&'reply str>,
    tool_calls: Option<&'reply [ReplyToolCall]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<&'reply [Annotation]>,
}

fn logged_model_response(reply: &CohereV2Reply) -> Result<Json, PassthroughLoggingError> {
    let message = &reply.message;
    let logged = LoggedResponse {
        id: &reply.id,
        object: "chat.completion",
        created: reply.created,
        model: &reply.model,
        system_fingerprint: None,
        // The default Choices carries finish_reason "stop": transform_response never sets one.
        choices: [LoggedChoice {
            finish_reason: reply.finish_reason.as_deref().unwrap_or("stop"),
            index: 0,
            message: LoggedMessage {
                content: message.content.as_deref(),
                role: "assistant",
                function_call: None,
                tool_calls: message.tool_calls.as_deref(),
                annotations: message.annotations.as_deref(),
            },
        }],
        usage: &reply.usage,
    };
    let encoded = serde_json::to_string(&logged)
        .map_err(|source| PassthroughStage::Transform.failed(source))?;
    serde_json::from_str(&encoded).map_err(|source| PassthroughStage::Transform.failed(source))
}
