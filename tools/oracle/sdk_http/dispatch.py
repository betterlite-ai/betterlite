import contextlib
import sys

from pydantic import BaseModel, ConfigDict

from anthropic_beta_config import BetaConfigCall, run_beta_config
from openai_chat import ChatCall, run_chat
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


class AnthropicBetaConfigCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    anthropic_beta_config: BetaConfigCall


class Request(BaseModel):
    model_config = ConfigDict(extra="forbid")

    call: (
        OpenaiChatCall
        | VectorStoreEmbeddingExecutorCall
        | VectorQueryEmbeddingCall
        | AnthropicBetaConfigCall
    )
    api_base: str


def main() -> None:
    raw = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
    if len(raw) > MAX_REQUEST_BYTES:
        raise RequestTooLargeError
    request = Request.model_validate_json(raw)
    with contextlib.redirect_stdout(sys.stderr):
        output: str
        if isinstance(request.call, OpenaiChatCall):
            output = run_chat(request.call.openai_chat, request.api_base).model_dump_json(
                by_alias=True, exclude_unset=True
            )
        elif isinstance(request.call, AnthropicBetaConfigCall):
            output = run_beta_config(request.call.anthropic_beta_config, request.api_base)
        elif isinstance(request.call, VectorQueryEmbeddingCall):
            output = run_query(
                request.call.vector_query_embedding, request.api_base
            ).model_dump_json(by_alias=True, exclude_unset=True)
        else:
            output = run_embedding(
                request.call.vector_store_embedding_executor, request.api_base
            ).model_dump_json(by_alias=True, exclude_unset=True)
    sys.stdout.write(output)


if __name__ == "__main__":
    main()
