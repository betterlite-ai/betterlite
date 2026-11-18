use serde::{Deserialize, Serialize};

use super::{
    AgenticContinuationRun, BetaConfigRun, ChatCompletion, ChatResponseStreamRuns, ChatStreamRun,
    EmbeddingOutcomes, Failure, FileStorageRun, HeaderStreamRun, QueryVectors, StreamFactoryRun,
    SyncRun,
};
use crate::sdk_http::messages_stream_logging::outcome::StreamLoggingRun;
use crate::sdk_http::messages_stream_logging::relay::call::StepOutcome;
use crate::sdk_http::{
    assistant_create, assistant_dispatch, assistant_messages, assistant_read_delete, assistant_run,
    assistant_threads, compression_interception, context_edit_dispatch, decisions_send,
    file_content_stream, file_create_list_transport, file_http_content, file_http_metadata,
    file_http_upload, file_provider_read, file_provider_write, file_storage_service,
    file_stream_logging, file_upload_routing, fine_tuning_cancel, fine_tuning_create,
    fine_tuning_read, image_edit_call, image_edit_http, image_generation_call,
    image_generation_http, image_generation_plan, image_variations, messages_chat_call,
    messages_chat_transform_stream, messages_context_preparation, messages_native_execute,
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

// A task-built kind's outcome is boxed, so the type its task writes never changes this enum's size
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SdkResult {
    Success {
        value: ChatCompletion,
    },
    Embedding {
        value: EmbeddingOutcomes,
    },
    Vectors {
        value: QueryVectors,
    },
    BetaConfig {
        value: BetaConfigRun,
    },
    MessagesHeaderStream {
        value: HeaderStreamRun,
    },
    MessagesStreamFactory {
        value: StreamFactoryRun,
    },
    MessagesStreamLogging {
        value: StreamLoggingRun,
    },
    MessagesNativeStream {
        value: Vec<StepOutcome>,
    },
    MessagesSyncStream {
        value: SyncRun,
    },
    MessagesChatStream {
        value: ChatStreamRun,
    },
    ChatResponseStream {
        value: ChatResponseStreamRuns,
    },
    ResponsesAgenticContinuation {
        value: AgenticContinuationRun,
    },
    FileAzureStorage {
        value: FileStorageRun,
    },
    AssistantCreate {
        value: Box<assistant_create::Outcome>,
    },
    AssistantDispatch {
        value: Box<assistant_dispatch::Outcome>,
    },
    AssistantMessages {
        value: Box<assistant_messages::Outcome>,
    },
    AssistantReadDelete {
        value: Box<assistant_read_delete::Outcome>,
    },
    AssistantRun {
        value: Box<assistant_run::Outcome>,
    },
    AssistantThreads {
        value: Box<assistant_threads::Outcome>,
    },
    CompressionInterception {
        value: Box<compression_interception::Outcome>,
    },
    ContextEditDispatch {
        value: Box<context_edit_dispatch::Outcome>,
    },
    DecisionsSend {
        value: Box<decisions_send::Outcome>,
    },
    FileContentStream {
        value: Box<file_content_stream::Outcome>,
    },
    FileHttpContent {
        value: Box<file_http_content::Outcome>,
    },
    FileHttpMetadata {
        value: Box<file_http_metadata::Outcome>,
    },
    FileHttpUpload {
        value: Box<file_http_upload::Outcome>,
    },
    FileProviderRead {
        value: Box<file_provider_read::Outcome>,
    },
    FileProviderWrite {
        value: Box<file_provider_write::Outcome>,
    },
    FileStorageService {
        value: Box<file_storage_service::Outcome>,
    },
    FileStreamLogging {
        value: Box<file_stream_logging::Outcome>,
    },
    FileUploadRouting {
        value: Box<file_upload_routing::Outcome>,
    },
    FineTuningCancel {
        value: Box<fine_tuning_cancel::Outcome>,
    },
    FineTuningCreate {
        value: Box<fine_tuning_create::Outcome>,
    },
    FineTuningRead {
        value: Box<fine_tuning_read::Outcome>,
    },
    ImageEditCall {
        value: Box<image_edit_call::Outcome>,
    },
    ImageEditHttp {
        value: Box<image_edit_http::Outcome>,
    },
    ImageGenerationCall {
        value: Box<image_generation_call::Outcome>,
    },
    ImageGenerationHttp {
        value: Box<image_generation_http::Outcome>,
    },
    ImageGenerationPlan {
        value: Box<image_generation_plan::Outcome>,
    },
    ImageVariations {
        value: Box<image_variations::Outcome>,
    },
    MessagesChatCall {
        value: Box<messages_chat_call::Outcome>,
    },
    MessagesChatTransformStream {
        value: Box<messages_chat_transform_stream::Outcome>,
    },
    MessagesContextPreparation {
        value: Box<messages_context_preparation::Outcome>,
    },
    MessagesNativeExecute {
        value: Box<messages_native_execute::Outcome>,
    },
    RealtimeHttpCalls {
        value: Box<realtime_http_calls::Outcome>,
    },
    RealtimeHttpTransport {
        value: Box<realtime_http_transport::Outcome>,
    },
    RerankCall {
        value: Box<rerank_call::Outcome>,
    },
    RerankHttp {
        value: Box<rerank_http::Outcome>,
    },
    RerankProviderPlan {
        value: Box<rerank_provider_plan::Outcome>,
    },
    ResponseChatDispatch {
        value: Box<response_chat_dispatch::Outcome>,
    },
    ResponsesAsyncCreate {
        value: Box<responses_async_create::Outcome>,
    },
    ResponsesChatCall {
        value: Box<responses_chat_call::Outcome>,
    },
    ResponsesCompact {
        value: Box<responses_compact::Outcome>,
    },
    ResponsesCompactTransport {
        value: Box<responses_compact_transport::Outcome>,
    },
    ResponsesCreate {
        value: Box<responses_create::Outcome>,
    },
    ResponsesCreateTransport {
        value: Box<responses_create_transport::Outcome>,
    },
    ResponsesLifecycleRead {
        value: Box<responses_lifecycle_read::Outcome>,
    },
    ResponsesLifecycleWrite {
        value: Box<responses_lifecycle_write::Outcome>,
    },
    ResponsesPollStream {
        value: Box<responses_poll_stream::Outcome>,
    },
    ResponsesPublicDispatch {
        value: Box<responses_public_dispatch::Outcome>,
    },
    ResponsesReadTransport {
        value: Box<responses_read_transport::Outcome>,
    },
    ResponsesStreamDriver {
        value: Box<responses_stream_driver::Outcome>,
    },
    ResponsesWriteTransport {
        value: Box<responses_write_transport::Outcome>,
    },
    SpeechCallPlan {
        value: Box<speech_call_plan::Outcome>,
    },
    SpeechCall {
        value: Box<speech_call::Outcome>,
    },
    SpeechChatBridge {
        value: Box<speech_chat_bridge::Outcome>,
    },
    SpeechHttp {
        value: Box<speech_http::Outcome>,
    },
    SummaryExecution {
        value: Box<summary_execution::Outcome>,
    },
    TranscriptionCall {
        value: Box<transcription_call::Outcome>,
    },
    TranscriptionHttp {
        value: Box<transcription_http::Outcome>,
    },
    VectorFileCreateListTransport {
        value: Box<file_create_list_transport::Outcome>,
    },
    VectorFileCreateList {
        value: Box<vector_file_create_list::Outcome>,
    },
    VectorFileReadTransport {
        value: Box<vector_file_read_transport::Outcome>,
    },
    VectorFileRead {
        value: Box<vector_file_read::Outcome>,
    },
    VectorFileWriteTransport {
        value: Box<vector_file_write_transport::Outcome>,
    },
    VectorFileWrite {
        value: Box<vector_file_write::Outcome>,
    },
    VectorRetrievalContext {
        value: Box<vector_retrieval_context::Outcome>,
    },
    VectorSearchTransport {
        value: Box<vector_search_transport::Outcome>,
    },
    VectorStoreCreateReadTransport {
        value: Box<store_create_read_transport::Outcome>,
    },
    VectorStoreRead {
        value: Box<vector_store_read::Outcome>,
    },
    VectorStoreSearchCreate {
        value: Box<vector_store_search_create::Outcome>,
    },
    VectorStoreTransport {
        value: Box<vector_store_transport::Outcome>,
    },
    VectorStoreWrite {
        value: Box<vector_store_write::Outcome>,
    },
    Failure(Failure),
}
