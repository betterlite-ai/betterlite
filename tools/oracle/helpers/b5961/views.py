from typing import Final, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

from .types import Object


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
