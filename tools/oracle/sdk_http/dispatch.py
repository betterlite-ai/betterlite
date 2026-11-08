import contextlib
import sys

from openai_chat import ChatCall, run_chat
from pydantic import BaseModel, ConfigDict
from vector_query_embedding import QueryEmbeddingCall, run_query
from vector_store_embedding_executor import EmbeddingCall, run_embedding

MAX_REQUEST_BYTES = 1048576


class RequestTooLargeError(ValueError):
    def __init__(self) -> None:
        super().__init__("the call is larger than the request limit")


class OpenaiChatCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    openai_chat: ChatCall


class VectorStoreEmbeddingExecutorCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_embedding_executor: EmbeddingCall


class VectorQueryEmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_query_embedding: QueryEmbeddingCall


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: OpenaiChatCall | VectorStoreEmbeddingExecutorCall | VectorQueryEmbeddingCall
    api_base: str


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestTooLargeError
    request = Request.model_validate_json(raw)
    with contextlib.redirect_stdout(sys.stderr):
        outcome: BaseModel
        if isinstance(request.call, OpenaiChatCall):
            outcome = run_chat(request.call.openai_chat, request.api_base)
        elif isinstance(request.call, VectorQueryEmbeddingCall):
            outcome = run_query(request.call.vector_query_embedding, request.api_base)
        else:
            outcome = run_embedding(
                request.call.vector_store_embedding_executor, request.api_base
            )
    sys.stdout.write(outcome.model_dump_json(by_alias=True, exclude_unset=True))


if __name__ == "__main__":
    main()
