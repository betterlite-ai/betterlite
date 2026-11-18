import contextlib
import os
import sys

from pydantic import BaseModel, ConfigDict, Field

from azure_file_storage import (
    DownloadCall,
    UploadCall,
    run_file_azure_storage,
)
from beta_config_call import BetaConfigCall, run_beta_config
from chat_response_stream import ChatResponseStreamCall, run_chat_response_stream
from messages_chat_stream import (
    MessagesChatStreamCall,
    run_messages_chat_stream,
)
from messages_header_stream import (
    MessagesHeaderStreamCall,
    run_messages_header_stream,
)
from messages_native_stream import (
    MessagesNativeStreamCall,
    run_messages_native_stream,
)
from messages_stream_factory import (
    MessagesStreamFactoryCall,
    run_messages_stream_factory,
)
from messages_stream_logging import (
    MessagesStreamLoggingCall,
    run_messages_stream_logging,
)
from messages_sync_stream import (
    MessagesSyncStreamCall,
    run_messages_sync_stream,
)
from openai_chat import ChatCall, run_chat
from query_embedding_call import QueryEmbeddingCall, run_query
from responses_agentic_continuation import ContinuationCall, run_continuation
from vector_store_embedding_executor import EmbeddingCall, run_embedding

MAX_REQUEST_BYTES = 1048576


class RequestTooLargeError(ValueError):
    def __init__(self) -> None:
        super().__init__("the call is larger than the request limit")


class OpenaiChatCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    openai_chat: ChatCall

    def serialize(self, api_base: str) -> str:
        return run_chat(self.openai_chat, api_base).model_dump_json(
            by_alias=True, exclude_unset=True
        )


class EmbeddingExecutorCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    embedding_executor: EmbeddingCall = Field(alias="vector_store_embedding_executor")

    def serialize(self, api_base: str) -> str:
        return run_embedding(self.embedding_executor, api_base).model_dump_json(
            by_alias=True, exclude_unset=True
        )


class VectorQueryEmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_query_embedding: QueryEmbeddingCall

    def serialize(self, api_base: str) -> str:
        return run_query(self.vector_query_embedding, api_base).model_dump_json(
            by_alias=True, exclude_unset=True
        )


class AnthropicBetaConfigCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    anthropic_beta_config: BetaConfigCall

    def serialize(self, api_base: str) -> str:
        return run_beta_config(self.anthropic_beta_config, api_base)


class MessagesHeaderStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_header_stream: MessagesHeaderStreamCall

    def serialize(self, api_base: str) -> str:
        return run_messages_header_stream(self.messages_header_stream, api_base)


class MessagesStreamFactoryRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_stream_factory: MessagesStreamFactoryCall

    def serialize(self, api_base: str) -> str:
        return run_messages_stream_factory(self.messages_stream_factory, api_base)


class MessagesStreamLoggingRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_stream_logging: MessagesStreamLoggingCall

    def serialize(self, api_base: str) -> str:
        return run_messages_stream_logging(self.messages_stream_logging, api_base)


class MessagesNativeStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_native_stream: MessagesNativeStreamCall

    def serialize(self, api_base: str) -> str:
        return run_messages_native_stream(self.messages_native_stream, api_base)


class MessagesSyncStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_sync_stream: MessagesSyncStreamCall

    def serialize(self, api_base: str) -> str:
        return run_messages_sync_stream(self.messages_sync_stream, api_base)


class MessagesChatStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_chat_stream: MessagesChatStreamCall

    def serialize(self, api_base: str) -> str:
        return run_messages_chat_stream(self.messages_chat_stream, api_base)


class ChatResponseStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    chat_response_stream: ChatResponseStreamCall

    def serialize(self, api_base: str) -> str:
        return run_chat_response_stream(self.chat_response_stream, api_base)


class ContinuationRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_agentic_continuation: ContinuationCall

    def serialize(self, api_base: str) -> str:
        return run_continuation(self.responses_agentic_continuation, api_base)


class FileAzureStorageRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_azure_storage: UploadCall | DownloadCall

    def serialize(self, api_base: str) -> str:
        return run_file_azure_storage(self.file_azure_storage, api_base)


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: (
        OpenaiChatCall
        | EmbeddingExecutorCall
        | VectorQueryEmbeddingCall
        | AnthropicBetaConfigCall
        | MessagesHeaderStreamRequest
        | MessagesStreamFactoryRequest
        | MessagesStreamLoggingRequest
        | MessagesNativeStreamRequest
        | MessagesSyncStreamRequest
        | MessagesChatStreamRequest
        | ChatResponseStreamRequest
        | ContinuationRequest
        | FileAzureStorageRequest
    )
    api_base: str


def read_request() -> bytes:
    with os.fdopen(0, "rb", closefd=False) as stream:
        raw = stream.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestTooLargeError
    return raw


def main() -> None:
    raw = read_request()
    request = Request.model_validate_json(raw)
    with contextlib.redirect_stdout(sys.stderr):
        output = request.call.serialize(request.api_base)
    sys.stdout.write(output)


if __name__ == "__main__":
    main()
