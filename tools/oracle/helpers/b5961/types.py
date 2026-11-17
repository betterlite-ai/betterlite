from typing import Literal, TypedDict

from litellm.types.llms.openai import AllEmbeddingInputValues
from pydantic import JsonValue

Object = dict[str, JsonValue]

Function = Literal[
    "model_config",
    "headers",
    "hf_request",
    "hf_reply",
    "cohere_map",
    "cohere_request",
    "cohere_reply",
    "decode_stream",
]


class ModelConfig(TypedDict):
    function: Literal["model_config"]
    model: str


class Headers(TypedDict):
    function: Literal["headers"]
    kind: Literal["hugging_face", "cohere"]
    headers: dict[str, str]


class HfRequest(TypedDict):
    function: Literal["hf_request"]
    input: AllEmbeddingInputValues
    optional_params: Object


class HfReply(TypedDict):
    function: Literal["hf_reply"]
    body: str
    status: int
    model: str
    request: Object


class CohereMap(TypedDict):
    function: Literal["cohere_map"]
    non_default: Object
    optional_params: Object


class CohereRequest(TypedDict):
    function: Literal["cohere_request"]
    model: str
    input: AllEmbeddingInputValues
    optional_params: Object


class CohereReply(TypedDict):
    function: Literal["cohere_reply"]
    model: str
    body: str
    input: JsonValue
    request_data: Object


class DecodeStream(TypedDict):
    function: Literal["decode_stream"]
    model: str
    is_messages_api: bool
    sync: bool
    chunks: list[str]
