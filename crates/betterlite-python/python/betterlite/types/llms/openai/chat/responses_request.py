from typing import Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.llms.openai.core.chat_blocks import PromptCacheOptions

# openai's ResponseIncludable, written out because the wheel does not depend on openai
ResponseIncludableValue: TypeAlias = Literal[
    "file_search_call.results",
    "web_search_call.results",
    "web_search_call.action.sources",
    "message.input_image.image_url",
    "computer_call_output.output.image_url",
    "code_interpreter_call.outputs",
    "reasoning.encrypted_content",
    "message.output_text.logprobs",
]


class ComputerToolParam(TypedDict, total=False):
    display_height: Required[float]
    display_width: Required[float]
    environment: Required[str]
    type: Required[str]


class ShellToolParam(TypedDict, total=False):
    type: Required[str]
    environment: Required[dict[str, JsonValue]]


ALL_RESPONSES_API_TOOL_PARAMS: TypeAlias = (
    dict[str, JsonValue] | ComputerToolParam | ShellToolParam
)


class PromptObject(TypedDict, total=False):
    id: Required[str]
    variables: dict[str, JsonValue] | None
    version: str | None


class ContextManagementEntry(TypedDict, total=False):
    type: str
    compact_threshold: int


class ResponsesAPIStreamOptions(TypedDict, total=False):
    include_obfuscation: bool


class ResponsesAPIOptionalRequestParams(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    include: list[ResponseIncludableValue] | None
    instructions: str | None
    max_output_tokens: int | None
    metadata: dict[str, JsonValue] | None
    parallel_tool_calls: bool | None
    previous_response_id: str | None
    reasoning: dict[str, JsonValue] | None
    store: bool | None
    background: bool | None
    stream: bool | None
    temperature: float | None
    text: dict[str, JsonValue] | None
    tool_choice: Literal["none", "auto", "required"] | dict[str, JsonValue] | None
    tools: list[ALL_RESPONSES_API_TOOL_PARAMS] | None
    top_p: float | None
    truncation: Literal["auto", "disabled"] | None
    user: str | None
    service_tier: str | None
    safety_identifier: str | None
    prompt: PromptObject | None
    max_tool_calls: int | None
    prompt_cache_key: str | None
    prompt_cache_retention: str | None
    prompt_cache_options: ReadOnly[PromptCacheOptions | None]
    stream_options: ResponsesAPIStreamOptions | None
    top_logprobs: int | None
    partial_images: int | None
    context_management: list[ContextManagementEntry] | None


# ast-grep-ignore: no-long-names
class ResponsesAPIRequestParams(ResponsesAPIOptionalRequestParams, total=False):
    input: Required[ReadOnly[str | list[dict[str, JsonValue]]]]
    model: Required[ReadOnly[str]]
