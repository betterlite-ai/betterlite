from typing import TypeAlias

import litellm
from pydantic import JsonValue

from .chat import (
    ChatMapInput,
    ChatSupportedInput,
    EmbeddingMapInput,
    chat_map,
    chat_supported,
    embedding_map,
)
from .passthrough import (
    PassthroughCredentialsInput,
    PassthroughUrlInput,
    passthrough_credentials,
    passthrough_url,
)
from .rerank import (
    RerankParamsInput,
    RerankRequestInput,
    RerankResponseInput,
    RerankSelectInput,
    rerank_params,
    rerank_request,
    rerank_response,
    rerank_select,
)

__all__ = ["litellm", "run"]

Input: TypeAlias = (
    ChatSupportedInput
    | ChatMapInput
    | EmbeddingMapInput
    | RerankSelectInput
    | RerankParamsInput
    | RerankRequestInput
    | RerankResponseInput
    | PassthroughUrlInput
    | PassthroughCredentialsInput
)


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    if input_value["kind"] == "chat_supported":
        result = chat_supported(input_value)
    elif input_value["kind"] == "chat_map":
        result = chat_map(input_value)
    elif input_value["kind"] == "embedding_map":
        result = embedding_map(input_value)
    elif input_value["kind"] == "rerank_select":
        result = rerank_select(input_value)
    elif input_value["kind"] == "rerank_params":
        result = rerank_params(input_value)
    elif input_value["kind"] == "rerank_request":
        result = rerank_request(input_value)
    elif input_value["kind"] == "rerank_response":
        result = rerank_response(input_value)
    elif input_value["kind"] == "passthrough_url":
        result = passthrough_url(input_value)
    else:
        result = passthrough_credentials(input_value)
    return result
