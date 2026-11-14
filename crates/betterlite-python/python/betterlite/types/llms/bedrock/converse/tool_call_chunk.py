from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class ChatCompletionToolCallFunctionChunk(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    name: str | None
    arguments: str
    provider_specific_fields: dict[str, JsonValue] | None


class ChatCompletionToolCallChunk(TypedDict):
    id: str | None
    type: Literal["function"]
    function: ChatCompletionToolCallFunctionChunk  # ast-grep-ignore: no-long-names
    index: int
