use std::collections::BTreeMap;

use super::call_kind::CallKind as _;
use super::calls::SdkHttpCall;
use super::capture::{Execution, Stop, run_until_stopped};
use super::responses_agentic_continuation as agentic;
use super::{
    anthropic_beta_config, chat_response_stream, embedding_executor, file_azure_storage,
    messages_chat_stream, messages_header_stream, messages_stream_logging, messages_sync_stream,
    native, vector_query_embedding,
};

#[expect(
    clippy::too_many_lines,
    reason = "one arm per call kind, and the set of call kinds is closed"
)]
pub(crate) fn execute(
    call: &SdkHttpCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
    stop: &Stop<'_>,
) -> Execution {
    match call {
        SdkHttpCall::OpenaiChat(chat) => run_until_stopped(native::send(chat, api_base, env), stop),
        SdkHttpCall::VectorStoreEmbeddingExecutor(embedding) => {
            run_until_stopped(embedding_executor::send(embedding, api_base, env), stop)
        }
        SdkHttpCall::VectorQueryEmbedding(query) => {
            run_until_stopped(vector_query_embedding::send(query, api_base, env), stop)
        }
        SdkHttpCall::AnthropicBetaConfig(beta) => {
            run_until_stopped(anthropic_beta_config::send(beta, api_base, env), stop)
        }
        SdkHttpCall::MessagesHeaderStream(stream) => {
            run_until_stopped(messages_header_stream::send(stream, api_base, env), stop)
        }
        SdkHttpCall::MessagesStreamLogging(logging) => {
            run_until_stopped(messages_stream_logging::send(logging, api_base, env), stop)
        }
        SdkHttpCall::MessagesNativeStream(relay) => run_until_stopped(
            messages_stream_logging::relay::send(relay, api_base, env),
            stop,
        ),
        SdkHttpCall::MessagesSyncStream(stream) => {
            run_until_stopped(messages_sync_stream::send(stream, api_base, env), stop)
        }
        SdkHttpCall::MessagesChatStream(stream) => {
            run_until_stopped(messages_chat_stream::send(stream, api_base, env), stop)
        }
        SdkHttpCall::ChatResponseStream(stream) => {
            run_until_stopped(chat_response_stream::send(stream, api_base, env), stop)
        }
        SdkHttpCall::ResponsesAgenticContinuation(continuation) => {
            run_until_stopped(agentic::send(continuation, api_base, env), stop)
        }
        SdkHttpCall::FileAzureStorage(storage) => {
            run_until_stopped(file_azure_storage::send(storage, api_base, env), stop)
        }
        SdkHttpCall::AssistantCreate(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::AssistantDispatch(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::AssistantMessages(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::AssistantReadDelete(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::AssistantRun(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::AssistantThreads(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::CompressionInterception(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ContextEditDispatch(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::DecisionsSend(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileContentStream(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileHttpContent(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileHttpMetadata(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileHttpUpload(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileProviderRead(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileProviderWrite(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileStorageService(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileStreamLogging(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FileUploadRouting(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FineTuningCancel(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FineTuningCreate(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::FineTuningRead(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageEditCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageEditHttp(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageGenerationCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageGenerationHttp(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageGenerationPlan(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ImageVariations(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::MessagesChatCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::MessagesChatTransformStream(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::MessagesContextPreparation(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::MessagesNativeExecute(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::RealtimeHttpCalls(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::RealtimeHttpTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::RerankCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::RerankHttp(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::RerankProviderPlan(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ResponseChatDispatch(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesAsyncCreate(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesChatCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ResponsesCompact(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ResponsesCompactTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesCreate(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ResponsesCreateTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesLifecycleRead(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesLifecycleWrite(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesPollStream(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::ResponsesPublicDispatch(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesReadTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesStreamDriver(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::ResponsesWriteTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::SpeechCallPlan(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::SpeechCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::SpeechChatBridge(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::SpeechHttp(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::SummaryExecution(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::TranscriptionCall(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::TranscriptionHttp(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::VectorFileCreateListTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorFileCreateList(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorFileReadTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorFileRead(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::VectorFileWriteTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorFileWrite(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::VectorRetrievalContext(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorSearchTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorStoreCreateReadTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorStoreRead(kind) => run_until_stopped(kind.send(api_base, env), stop),
        SdkHttpCall::VectorStoreSearchCreate(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorStoreTransport(kind) => {
            run_until_stopped(kind.send(api_base, env), stop)
        }
        SdkHttpCall::VectorStoreWrite(kind) => run_until_stopped(kind.send(api_base, env), stop),
    }
}
