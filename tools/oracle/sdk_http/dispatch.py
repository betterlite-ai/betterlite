import contextlib
import os
import sys

from pydantic import BaseModel, ConfigDict, Field

from beta_config_call import BetaConfigCall, run_beta_config
from messages_header_stream import (
    MessagesHeaderStreamCall,
    run_messages_header_stream,
)
from openai_chat import ChatCall, run_chat
from query_embedding_call import QueryEmbeddingCall, run_query
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


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: (
        OpenaiChatCall
        | EmbeddingExecutorCall
        | VectorQueryEmbeddingCall
        | AnthropicBetaConfigCall
        | MessagesHeaderStreamRequest
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
