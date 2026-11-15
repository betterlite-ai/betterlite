from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class FunctionCall(TypedDict):
    name: str | None
    arguments: str | dict[str, JsonValue] | None


class MistralToolCallMessage(TypedDict):
    id: str | None
    type: Literal["function"]
    function: FunctionCall | None


class MistralTextBlock(TypedDict):
    type: Literal["text"]
    text: str


class MistralThinkingBlock(TypedDict):
    type: Literal["thinking"]
    thinking: list[MistralTextBlock]
