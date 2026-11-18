use serde::{Deserialize, Serialize};

use betterlite_models::decisions::Json;

use super::call_checks::validate_timeout;
use super::call_kind::CallKind as _;
use super::chat_response_stream::call::ChatResponseStreamCall;
use super::error::SdkHttpError;
use super::file_azure_storage::FileAzureStorageCall;
use super::messages_chat_stream::call::MessagesChatStreamCall;
use super::messages_stream_factory::MessagesStreamFactoryCall;
use super::messages_stream_logging::call::MessagesStreamLoggingCall;
use super::messages_stream_logging::relay::call::MessagesNativeStreamCall;
use super::messages_sync_stream::call;
use super::responses_agentic_continuation::AgenticContinuationCall;
use super::{
    AnthropicBetaConfigCall, MessagesHeaderStreamCall, OpenaiChat, OracleLimits,
    VectorQueryEmbeddingCall, VectorStoreEmbeddingCall, assistant_create, assistant_dispatch,
    assistant_messages, assistant_read_delete, assistant_run, assistant_threads,
    compression_interception, context_edit_dispatch, decisions_send, file_content_stream,
    file_create_list_transport, file_http_content, file_http_metadata, file_http_upload,
    file_provider_read, file_provider_write, file_storage_service, file_stream_logging,
    file_upload_routing, fine_tuning_cancel, fine_tuning_create, fine_tuning_read, image_edit_call,
    image_edit_http, image_generation_call, image_generation_http, image_generation_plan,
    image_variations, messages_chat_call, messages_chat_stream, messages_chat_transform_stream,
    messages_context_preparation, messages_native_execute, messages_stream_factory,
    realtime_http_calls, realtime_http_transport, rerank_call, rerank_http, rerank_provider_plan,
    response_chat_dispatch, responses_async_create, responses_chat_call, responses_compact,
    responses_compact_transport, responses_create, responses_create_transport,
    responses_lifecycle_read, responses_lifecycle_write, responses_poll_stream,
    responses_public_dispatch, responses_read_transport, responses_stream_driver,
    responses_write_transport, speech_call, speech_call_plan, speech_chat_bridge, speech_http,
    store_create_read_transport, summary_execution, transcription_call, transcription_http,
    vector_file_create_list, vector_file_read, vector_file_read_transport, vector_file_write,
    vector_file_write_transport, vector_retrieval_context, vector_search_transport,
    vector_store_read, vector_store_search_create, vector_store_transport, vector_store_write,
};

// A task-built call kind is boxed, so the call type its task writes never changes this enum's size
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkHttpCall {
    OpenaiChat(OpenaiChat),
    VectorStoreEmbeddingExecutor(VectorStoreEmbeddingCall),
    VectorQueryEmbedding(VectorQueryEmbeddingCall),
    AnthropicBetaConfig(AnthropicBetaConfigCall),
    MessagesHeaderStream(MessagesHeaderStreamCall),
    MessagesStreamFactory(MessagesStreamFactoryCall),
    MessagesStreamLogging(MessagesStreamLoggingCall),
    MessagesNativeStream(MessagesNativeStreamCall),
    MessagesSyncStream(Box<call::MessagesSyncStreamCall>),
    MessagesChatStream(Box<MessagesChatStreamCall>),
    ChatResponseStream(Box<ChatResponseStreamCall>),
    ResponsesAgenticContinuation(Box<AgenticContinuationCall>),
    FileAzureStorage(FileAzureStorageCall),
    AssistantCreate(Box<assistant_create::Call>),
    AssistantDispatch(Box<assistant_dispatch::Call>),
    AssistantMessages(Box<assistant_messages::Call>),
    AssistantReadDelete(Box<assistant_read_delete::Call>),
    AssistantRun(Box<assistant_run::Call>),
    AssistantThreads(Box<assistant_threads::Call>),
    CompressionInterception(Box<compression_interception::Call>),
    ContextEditDispatch(Box<context_edit_dispatch::Call>),
    DecisionsSend(Box<decisions_send::Call>),
    FileContentStream(Box<file_content_stream::Call>),
    FileHttpContent(Box<file_http_content::Call>),
    FileHttpMetadata(Box<file_http_metadata::Call>),
    FileHttpUpload(Box<file_http_upload::Call>),
    FileProviderRead(Box<file_provider_read::Call>),
    FileProviderWrite(Box<file_provider_write::Call>),
    FileStorageService(Box<file_storage_service::Call>),
    FileStreamLogging(Box<file_stream_logging::Call>),
    FileUploadRouting(Box<file_upload_routing::Call>),
    FineTuningCancel(Box<fine_tuning_cancel::Call>),
    FineTuningCreate(Box<fine_tuning_create::Call>),
    FineTuningRead(Box<fine_tuning_read::Call>),
    ImageEditCall(Box<image_edit_call::Call>),
    ImageEditHttp(Box<image_edit_http::Call>),
    ImageGenerationCall(Box<image_generation_call::Call>),
    ImageGenerationHttp(Box<image_generation_http::Call>),
    ImageGenerationPlan(Box<image_generation_plan::Call>),
    ImageVariations(Box<image_variations::Call>),
    MessagesChatCall(Box<messages_chat_call::Call>),
    MessagesChatTransformStream(Box<messages_chat_transform_stream::Call>),
    MessagesContextPreparation(Box<messages_context_preparation::Call>),
    MessagesNativeExecute(Box<messages_native_execute::Call>),
    RealtimeHttpCalls(Box<realtime_http_calls::Call>),
    RealtimeHttpTransport(Box<realtime_http_transport::Call>),
    RerankCall(Box<rerank_call::Call>),
    RerankHttp(Box<rerank_http::Call>),
    RerankProviderPlan(Box<rerank_provider_plan::Call>),
    ResponseChatDispatch(Box<response_chat_dispatch::Call>),
    ResponsesAsyncCreate(Box<responses_async_create::Call>),
    ResponsesChatCall(Box<responses_chat_call::Call>),
    ResponsesCompact(Box<responses_compact::Call>),
    ResponsesCompactTransport(Box<responses_compact_transport::Call>),
    ResponsesCreate(Box<responses_create::Call>),
    ResponsesCreateTransport(Box<responses_create_transport::Call>),
    ResponsesLifecycleRead(Box<responses_lifecycle_read::Call>),
    ResponsesLifecycleWrite(Box<responses_lifecycle_write::Call>),
    ResponsesPollStream(Box<responses_poll_stream::Call>),
    ResponsesPublicDispatch(Box<responses_public_dispatch::Call>),
    ResponsesReadTransport(Box<responses_read_transport::Call>),
    ResponsesStreamDriver(Box<responses_stream_driver::Call>),
    ResponsesWriteTransport(Box<responses_write_transport::Call>),
    SpeechCallPlan(Box<speech_call_plan::Call>),
    SpeechCall(Box<speech_call::Call>),
    SpeechChatBridge(Box<speech_chat_bridge::Call>),
    SpeechHttp(Box<speech_http::Call>),
    SummaryExecution(Box<summary_execution::Call>),
    TranscriptionCall(Box<transcription_call::Call>),
    TranscriptionHttp(Box<transcription_http::Call>),
    VectorFileCreateListTransport(Box<file_create_list_transport::Call>),
    VectorFileCreateList(Box<vector_file_create_list::Call>),
    VectorFileReadTransport(Box<vector_file_read_transport::Call>),
    VectorFileRead(Box<vector_file_read::Call>),
    VectorFileWriteTransport(Box<vector_file_write_transport::Call>),
    VectorFileWrite(Box<vector_file_write::Call>),
    VectorRetrievalContext(Box<vector_retrieval_context::Call>),
    VectorSearchTransport(Box<vector_search_transport::Call>),
    VectorStoreCreateReadTransport(Box<store_create_read_transport::Call>),
    VectorStoreRead(Box<vector_store_read::Call>),
    VectorStoreSearchCreate(Box<vector_store_search_create::Call>),
    VectorStoreTransport(Box<vector_store_transport::Call>),
    VectorStoreWrite(Box<vector_store_write::Call>),
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per call kind, and the set of call kinds is closed"
)]
pub(crate) fn validate_call(call: &SdkHttpCall, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    match call {
        SdkHttpCall::OpenaiChat(chat) => validate_timeout(chat.timeout_seconds, limits)?,
        SdkHttpCall::VectorStoreEmbeddingExecutor(embedding) => {
            for deployment in embedding.router.iter().flatten() {
                if !deployment.litellm_params.model.starts_with("openai/") {
                    return Err(SdkHttpError::Invalid(
                        "every router deployment must serve an openai/ model".to_owned(),
                    ));
                }
            }
        }
        SdkHttpCall::MessagesHeaderStream(stream) => {
            if !matches!(stream.body, Json::Object(_)) {
                return Err(SdkHttpError::Invalid(
                    "the Messages header stream body must be an object".to_owned(),
                ));
            }
        }
        SdkHttpCall::MessagesStreamFactory(factory) => messages_stream_factory::validate(factory)?,
        SdkHttpCall::MessagesNativeStream(relay) => {
            if relay.queue_maxsize == 0 {
                return Err(SdkHttpError::Invalid(
                    "the relay queue maxsize must be greater than 0".to_owned(),
                ));
            }
        }
        SdkHttpCall::MessagesSyncStream(stream) => call::validate(stream, limits)?,
        SdkHttpCall::MessagesChatStream(stream) => {
            messages_chat_stream::call::validate(stream, limits)?;
        }
        SdkHttpCall::ChatResponseStream(stream) => {
            validate_timeout(stream.timeout_seconds, limits)?;
        }
        SdkHttpCall::ResponsesAgenticContinuation(_)
        | SdkHttpCall::VectorQueryEmbedding(_)
        | SdkHttpCall::AnthropicBetaConfig(_)
        | SdkHttpCall::MessagesStreamLogging(_)
        | SdkHttpCall::FileAzureStorage(_) => {}
        SdkHttpCall::AssistantCreate(kind) => kind.validate(limits)?,
        SdkHttpCall::AssistantDispatch(kind) => kind.validate(limits)?,
        SdkHttpCall::AssistantMessages(kind) => kind.validate(limits)?,
        SdkHttpCall::AssistantReadDelete(kind) => kind.validate(limits)?,
        SdkHttpCall::AssistantRun(kind) => kind.validate(limits)?,
        SdkHttpCall::AssistantThreads(kind) => kind.validate(limits)?,
        SdkHttpCall::CompressionInterception(kind) => kind.validate(limits)?,
        SdkHttpCall::ContextEditDispatch(kind) => kind.validate(limits)?,
        SdkHttpCall::DecisionsSend(kind) => kind.validate(limits)?,
        SdkHttpCall::FileContentStream(kind) => kind.validate(limits)?,
        SdkHttpCall::FileHttpContent(kind) => kind.validate(limits)?,
        SdkHttpCall::FileHttpMetadata(kind) => kind.validate(limits)?,
        SdkHttpCall::FileHttpUpload(kind) => kind.validate(limits)?,
        SdkHttpCall::FileProviderRead(kind) => kind.validate(limits)?,
        SdkHttpCall::FileProviderWrite(kind) => kind.validate(limits)?,
        SdkHttpCall::FileStorageService(kind) => kind.validate(limits)?,
        SdkHttpCall::FileStreamLogging(kind) => kind.validate(limits)?,
        SdkHttpCall::FileUploadRouting(kind) => kind.validate(limits)?,
        SdkHttpCall::FineTuningCancel(kind) => kind.validate(limits)?,
        SdkHttpCall::FineTuningCreate(kind) => kind.validate(limits)?,
        SdkHttpCall::FineTuningRead(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageEditCall(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageEditHttp(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageGenerationCall(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageGenerationHttp(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageGenerationPlan(kind) => kind.validate(limits)?,
        SdkHttpCall::ImageVariations(kind) => kind.validate(limits)?,
        SdkHttpCall::MessagesChatCall(kind) => kind.validate(limits)?,
        SdkHttpCall::MessagesChatTransformStream(kind) => kind.validate(limits)?,
        SdkHttpCall::MessagesContextPreparation(kind) => kind.validate(limits)?,
        SdkHttpCall::MessagesNativeExecute(kind) => kind.validate(limits)?,
        SdkHttpCall::RealtimeHttpCalls(kind) => kind.validate(limits)?,
        SdkHttpCall::RealtimeHttpTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::RerankCall(kind) => kind.validate(limits)?,
        SdkHttpCall::RerankHttp(kind) => kind.validate(limits)?,
        SdkHttpCall::RerankProviderPlan(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponseChatDispatch(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesAsyncCreate(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesChatCall(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesCompact(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesCompactTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesCreate(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesCreateTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesLifecycleRead(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesLifecycleWrite(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesPollStream(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesPublicDispatch(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesReadTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesStreamDriver(kind) => kind.validate(limits)?,
        SdkHttpCall::ResponsesWriteTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::SpeechCallPlan(kind) => kind.validate(limits)?,
        SdkHttpCall::SpeechCall(kind) => kind.validate(limits)?,
        SdkHttpCall::SpeechChatBridge(kind) => kind.validate(limits)?,
        SdkHttpCall::SpeechHttp(kind) => kind.validate(limits)?,
        SdkHttpCall::SummaryExecution(kind) => kind.validate(limits)?,
        SdkHttpCall::TranscriptionCall(kind) => kind.validate(limits)?,
        SdkHttpCall::TranscriptionHttp(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileCreateListTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileCreateList(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileReadTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileRead(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileWriteTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorFileWrite(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorRetrievalContext(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorSearchTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorStoreCreateReadTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorStoreRead(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorStoreSearchCreate(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorStoreTransport(kind) => kind.validate(limits)?,
        SdkHttpCall::VectorStoreWrite(kind) => kind.validate(limits)?,
    }
    Ok(())
}
