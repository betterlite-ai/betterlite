import contextlib
import os
import sys

from pydantic import BaseModel, ConfigDict, Field

from beta_config_call import BetaConfigCall, run_beta_config
from file_azure_storage import (
    DownloadCall,
    UploadCall,
    run_file_azure_storage,
)
from messages_header_stream import (
    MessagesHeaderStreamCall,
    run_messages_header_stream,
)
from messages_native_stream import (
    MessagesNativeStreamCall,
    run_messages_native_stream,
)
from messages_stream_logging import (
    MessagesStreamLoggingCall,
    run_messages_stream_logging,
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


class EmbeddingExecutorCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    embedding_executor: EmbeddingCall = Field(alias="vector_store_embedding_executor")


class VectorQueryEmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_query_embedding: QueryEmbeddingCall


class AnthropicBetaConfigCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    anthropic_beta_config: BetaConfigCall


class MessagesHeaderStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_header_stream: MessagesHeaderStreamCall


class MessagesStreamLoggingRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_stream_logging: MessagesStreamLoggingCall


class MessagesNativeStreamRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_native_stream: MessagesNativeStreamCall


class ContinuationRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_agentic_continuation: ContinuationCall


class FileAzureStorageRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    file_azure_storage: UploadCall | DownloadCall


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: (
        OpenaiChatCall
        | EmbeddingExecutorCall
        | VectorQueryEmbeddingCall
        | AnthropicBetaConfigCall
        | MessagesHeaderStreamRequest
        | MessagesStreamLoggingRequest
        | MessagesNativeStreamRequest
        | ContinuationRequest
        | FileAzureStorageRequest
    )
    api_base: str


def read_request() -> bytes:
    with os.fdopen(0, "rb", closefd=False) as stream:
        return stream.read(MAX_REQUEST_BYTES + 1)


def main() -> None:
    raw = read_request()
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestTooLargeError
    request = Request.model_validate_json(raw)
    with contextlib.redirect_stdout(sys.stderr):
        output: str
        match request.call:
            case OpenaiChatCall(openai_chat=chat):
                output = run_chat(chat, request.api_base).model_dump_json(
                    by_alias=True, exclude_unset=True
                )
            case AnthropicBetaConfigCall(anthropic_beta_config=beta):
                output = run_beta_config(beta, request.api_base)
            case MessagesHeaderStreamRequest(messages_header_stream=stream):
                output = run_messages_header_stream(stream, request.api_base)
            case MessagesStreamLoggingRequest(messages_stream_logging=logging_call):
                output = run_messages_stream_logging(logging_call, request.api_base)
            case MessagesNativeStreamRequest(messages_native_stream=relay_call):
                output = run_messages_native_stream(relay_call, request.api_base)
            case ContinuationRequest(responses_agentic_continuation=continuation):
                output = run_continuation(continuation, request.api_base)
            case FileAzureStorageRequest(file_azure_storage=storage_call):
                output = run_file_azure_storage(storage_call, request.api_base)
            case VectorQueryEmbeddingCall(vector_query_embedding=query):
                output = run_query(query, request.api_base).model_dump_json(
                    by_alias=True, exclude_unset=True
                )
            case EmbeddingExecutorCall(embedding_executor=embedding):
                output = run_embedding(embedding, request.api_base).model_dump_json(
                    by_alias=True, exclude_unset=True
                )
    sys.stdout.write(output)


if __name__ == "__main__":
    main()
