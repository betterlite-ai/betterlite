from typing import Final, Literal, NotRequired, TypedDict

from litellm.types.llms.openai import AllEmbeddingInputValues
from pydantic import JsonValue, TypeAdapter

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


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class HfRowView(TypedDict):
    object: str
    index: int
    embedding: JsonValue


class HfReplyView(TypedDict):
    object: str
    data: list[HfRowView]
    model: str
    usage: UsageView


class CohereRowView(TypedDict):
    object: str
    index: int
    embedding: JsonValue
    type: NotRequired[str]


class CohereReplyView(TypedDict):
    object: str
    data: list[CohereRowView]
    model: str
    usage: UsageView


HF_REPLY: Final = TypeAdapter(HfReplyView)
COHERE_REPLY: Final = TypeAdapter(CohereReplyView)


def usage_value(usage: UsageView) -> Object:
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
    }
