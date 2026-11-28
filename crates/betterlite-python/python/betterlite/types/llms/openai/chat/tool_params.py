from pydantic import JsonValue
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.openai.core.chat_blocks import ChatCompletionCachedContent


class ChatCompletionToolParamFunctionChunk(  # ast-grep-ignore: no-long-names
    TypedDict,
    total=False,
):
    name: Required[str]
    description: str
    parameters: dict[str, JsonValue]
    strict: bool
    eager_input_streaming: ReadOnly[bool]


class OpenAIChatCompletionToolParam(TypedDict):
    type: str
    function: ChatCompletionToolParamFunctionChunk  # ast-grep-ignore: no-long-names


class ChatCompletionToolParam(OpenAIChatCompletionToolParam, total=False):
    cache_control: ChatCompletionCachedContent
    allowed_callers: list[str]
    eager_input_streaming: ReadOnly[bool]
