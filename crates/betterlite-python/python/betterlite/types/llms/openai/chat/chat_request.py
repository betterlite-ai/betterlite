from collections.abc import Mapping

from pydantic import JsonValue
from typing_extensions import Required, TypedDict

from betterlite.types.llms.openai.chat.tool_choice import (
    ChatCompletionToolChoiceValues,
)
from betterlite.types.llms.openai.chat.tool_params import ChatCompletionToolParam
from betterlite.types.llms.openai.core.messages import AllMessageValues


class ChatCompletionRequest(TypedDict, total=False):
    model: Required[str]
    messages: Required[list[AllMessageValues]]
    frequency_penalty: float
    logit_bias: dict[str, JsonValue]
    logprobs: bool
    top_logprobs: int
    max_tokens: int
    n: int
    presence_penalty: float
    response_format: dict[str, JsonValue]
    seed: int
    service_tier: str
    safety_identifier: str
    prompt_cache_key: str
    stop: str | list[str]
    stream_options: dict[str, JsonValue]
    temperature: float
    top_p: float
    tools: list[ChatCompletionToolParam]
    tool_choice: ChatCompletionToolChoiceValues
    parallel_tool_calls: bool
    function_call: str | dict[str, JsonValue]
    functions: list[JsonValue]
    user: str
    metadata: dict[str, JsonValue]
    reasoning_effort: str
    output_config: Mapping[str, JsonValue]
