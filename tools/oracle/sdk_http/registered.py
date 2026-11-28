from typing import TypeAlias

from assistant_create import AssistantCreateRequest
from assistant_dispatch import AssistantDispatchRequest
from assistant_messages import AssistantMessagesRequest
from assistant_read_delete import AssistantReadDeleteRequest
from assistant_run import AssistantRunRequest
from assistant_threads import AssistantThreadsRequest
from compression_interception import CompressionInterceptionRequest
from context_edit_dispatch_call import ContextEditDispatchRequest
from decisions_send import DecisionsSendRequest
from file_content_stream import FileContentStreamRequest
from file_http_content import FileHttpContentRequest
from file_http_metadata import FileHttpMetadataRequest
from file_http_upload import FileHttpUploadRequest
from file_provider_read import FileProviderReadRequest
from file_provider_write import FileProviderWriteRequest
from file_stream_logging import FileStreamLoggingRequest
from file_upload_routing import FileUploadRoutingRequest
from fine_tuning_cancel import FineTuningCancelRequest
from fine_tuning_create import FineTuningCreateRequest
from fine_tuning_read import FineTuningReadRequest
from image_edit_call import ImageEditCallRequest
from image_edit_http import ImageEditHttpRequest
from image_generation_call import ImageGenerationCallRequest
from image_generation_http import ImageGenerationHttpRequest
from image_generation_plan import ImageGenerationPlanRequest
from image_variations import ImageVariationsRequest
from messages_chat_call import MessagesChatCallRequest
from messages_chat_transform_stream import ChatTransformStreamRequest
from messages_context_preparation import ContextPreparationRequest
from messages_native_execute import MessagesNativeExecuteRequest
from realtime_http_calls import RealtimeHttpCallsRequest
from realtime_http_transport import RealtimeHttpTransportRequest
from rerank_call import RerankCallRequest
from rerank_http import RerankHttpRequest
from rerank_provider_plan import RerankProviderPlanRequest
from response_chat_dispatch import ResponseChatDispatchRequest
from responses_async_create import ResponsesAsyncCreateRequest
from responses_chat_call import ResponsesChatCallRequest
from responses_compact import ResponsesCompactRequest
from responses_compact_transport import CompactTransportRequest
from responses_create import ResponsesCreateRequest
from responses_create_transport import CreateTransportRequest
from responses_lifecycle_read import ResponsesLifecycleReadRequest
from responses_lifecycle_write import ResponsesLifecycleWriteRequest
from responses_poll_stream import ResponsesPollStreamRequest
from responses_public_dispatch import ResponsesPublicDispatchRequest
from responses_read_transport import ResponsesReadTransportRequest
from responses_stream_driver import ResponsesStreamDriverRequest
from responses_write_transport import ResponsesWriteTransportRequest
from speech_call import SpeechCallRequest
from speech_call_plan import SpeechCallPlanRequest
from speech_chat_bridge import SpeechChatBridgeRequest
from speech_http import SpeechHttpRequest
from storage_backend_service import FileStorageServiceRequest
from summary_execution import SummaryExecutionRequest
from transcription_call import TranscriptionCallRequest
from transcription_http import TranscriptionHttpRequest
from vector_file_create_list import VectorFileCreateListRequest
from vector_file_create_list_transport import FileCreateListTransportRequest
from vector_file_read import VectorFileReadRequest
from vector_file_read_transport import VectorFileReadTransportRequest
from vector_file_write import VectorFileWriteRequest
from vector_file_write_transport import FileWriteTransportRequest
from vector_retrieval_context import VectorRetrievalContextRequest
from vector_search_transport import VectorSearchTransportRequest
from vector_store_create_read_transport import CreateReadTransportRequest
from vector_store_read import VectorStoreReadRequest
from vector_store_search_create import VectorStoreSearchCreateRequest
from vector_store_transport import VectorStoreTransportRequest
from vector_store_write import VectorStoreWriteRequest

RegisteredRequest: TypeAlias = (
    AssistantCreateRequest
    | AssistantDispatchRequest
    | AssistantMessagesRequest
    | AssistantReadDeleteRequest
    | AssistantRunRequest
    | AssistantThreadsRequest
    | CompressionInterceptionRequest
    | ContextEditDispatchRequest
    | DecisionsSendRequest
    | FileContentStreamRequest
    | FileHttpContentRequest
    | FileHttpMetadataRequest
    | FileHttpUploadRequest
    | FileProviderReadRequest
    | FileProviderWriteRequest
    | FileStorageServiceRequest
    | FileStreamLoggingRequest
    | FileUploadRoutingRequest
    | FineTuningCancelRequest
    | FineTuningCreateRequest
    | FineTuningReadRequest
    | ImageEditCallRequest
    | ImageEditHttpRequest
    | ImageGenerationCallRequest
    | ImageGenerationHttpRequest
    | ImageGenerationPlanRequest
    | ImageVariationsRequest
    | MessagesChatCallRequest
    | ChatTransformStreamRequest
    | ContextPreparationRequest
    | MessagesNativeExecuteRequest
    | RealtimeHttpCallsRequest
    | RealtimeHttpTransportRequest
    | RerankCallRequest
    | RerankHttpRequest
    | RerankProviderPlanRequest
    | ResponseChatDispatchRequest
    | ResponsesAsyncCreateRequest
    | ResponsesChatCallRequest
    | ResponsesCompactRequest
    | CompactTransportRequest
    | ResponsesCreateRequest
    | CreateTransportRequest
    | ResponsesLifecycleReadRequest
    | ResponsesLifecycleWriteRequest
    | ResponsesPollStreamRequest
    | ResponsesPublicDispatchRequest
    | ResponsesReadTransportRequest
    | ResponsesStreamDriverRequest
    | ResponsesWriteTransportRequest
    | SpeechCallPlanRequest
    | SpeechCallRequest
    | SpeechChatBridgeRequest
    | SpeechHttpRequest
    | SummaryExecutionRequest
    | TranscriptionCallRequest
    | TranscriptionHttpRequest
    | FileCreateListTransportRequest
    | VectorFileCreateListRequest
    | VectorFileReadTransportRequest
    | VectorFileReadRequest
    | FileWriteTransportRequest
    | VectorFileWriteRequest
    | VectorRetrievalContextRequest
    | VectorSearchTransportRequest
    | CreateReadTransportRequest
    | VectorStoreReadRequest
    | VectorStoreSearchCreateRequest
    | VectorStoreTransportRequest
    | VectorStoreWriteRequest
)
