from collections.abc import Mapping, Sequence
from typing import TypeAlias

from pydantic import JsonValue

from betterlite._cost.wire import JSON_OBJECT, JsonSource
from betterlite.types.decisions import DecisionsResponse, OpenAIDecisionResponse
from betterlite.types.rerank import RerankResponse
from betterlite.types.responses.api_response import ResponsesAPIResponse
from betterlite.types.utils.base import Dumpable, InitValue, ResponseObject
from betterlite.types.utils.embedding import EmbeddingResponse
from betterlite.types.utils.media import ImageResponse
from betterlite.types.utils.response import ModelResponseBase, ModelResponseStream
from betterlite.types.utils.text_completion import TextCompletionResponse
from betterlite.types.utils.transcription import TranscriptionResponse


def dump_object(value: JsonSource) -> dict[str, JsonValue]:
    match value:
        case Dumpable():
            return value.model_dump()
        case Mapping():
            return dict(value)


def source_value(value: JsonSource | None) -> dict[str, JsonValue] | None:
    if value is None:
        return None
    return dump_object(value)


# The response class decides how B068 reads its usage and which prices apply.
def response_kind(response: JsonSource) -> str:
    match response:
        case ModelResponseStream():
            return "chat_stream"
        case ModelResponseBase():
            return "chat"
        case EmbeddingResponse():
            return "embedding"
        case TranscriptionResponse():
            return "transcription"
        case RerankResponse():
            return "rerank"
        case ImageResponse():
            return "image"
        case TextCompletionResponse():
            return "text_completion"
        case ResponsesAPIResponse():
            return "responses"
        case Dumpable() | Mapping():
            return "other"


def hidden_object(value: InitValue) -> dict[str, JsonValue] | None:
    match value:
        case Dumpable():
            return value.model_dump()
        case Mapping():
            return JSON_OBJECT.validate_python(value)
        case str() | int() | float() | None | Sequence():
            return None


PrivateCarrier: TypeAlias = ResponseObject | DecisionsResponse | OpenAIDecisionResponse


# pydantic keeps private attributes beside fields; ResponseObject reads only fields.
def private_fields(response: PrivateCarrier) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = response.__pydantic_private__
    if private is None:
        return {}
    return private


# LiteLLM's hidden params, read from where pydantic keeps a private attribute.
def hidden_params(response: JsonSource) -> dict[str, JsonValue] | None:
    match response:
        case ResponseObject():
            return hidden_object(private_fields(response).get("_hidden_params"))
        case DecisionsResponse() | OpenAIDecisionResponse():
            return response.hidden_params
        case Dumpable() | Mapping():
            return None


# LiteLLM's hasattr(obj, "_hidden_params"), answered from the private bag.
def has_hidden_params(response: JsonSource) -> bool:
    match response:
        case DecisionsResponse() | OpenAIDecisionResponse() | ResponseObject():
            return "_hidden_params" in private_fields(response)
        case Dumpable() | Mapping():
            return False
