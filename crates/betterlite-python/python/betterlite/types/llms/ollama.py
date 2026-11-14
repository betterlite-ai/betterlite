from typing import Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class OllamaToolCallFunction(TypedDict):
    name: str
    arguments: dict[str, JsonValue]


class OllamaToolCall(TypedDict):
    function: OllamaToolCallFunction


class OllamaVisionModelObject(TypedDict):
    prompt: str
    images: list[str]


class OllamaChatCompletionMessage(TypedDict, total=False):
    role: Required[str]
    content: str
    thinking: str
    images: list[str]
    tool_calls: list[OllamaToolCall]
    tool_name: str
    tool_call_id: str
