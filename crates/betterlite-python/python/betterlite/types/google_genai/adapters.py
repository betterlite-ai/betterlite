from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.openai.chat.tool_choice import (
    ChatCompletionToolChoiceStringValues,
)
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.llms.openai.core.messages import AllMessageValues


class GenerateContentCompletionKwargs(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    model: str
    messages: list[AllMessageValues]
    temperature: float
    max_tokens: int
    top_p: float
    stop: str | list[str]
    tools: list[ChatCompletionToolParam]
    # ast-grep-ignore: no-long-names
    tool_choice: ChatCompletionToolChoiceStringValues
    stream: bool
    metadata: dict[str, JsonValue]
    extra_headers: dict[str, str] | None
    proxy_server_request: dict[str, JsonValue] | None
