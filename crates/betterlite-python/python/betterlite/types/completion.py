from collections.abc import Iterable
from typing import ClassVar, Literal, Required, TypeAlias, TypedDict, Union

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.values import JsonValue, StoredValue


# ast-grep-ignore: no-long-names
class ChatCompletionSystemMessageParam(TypedDict, total=False):
    content: Required[str]
    role: Required[Literal["system"]]
    name: str


# ast-grep-ignore: no-long-names
class ChatCompletionContentPartTextParam(TypedDict, total=False):
    text: Required[str]
    type: Required[Literal["text"]]


class ImageURL(TypedDict, total=False):
    url: Required[str]
    detail: Literal["auto", "low", "high"]


# ast-grep-ignore: no-long-names
class ChatCompletionContentPartImageParam(TypedDict, total=False):
    image_url: Required[ImageURL]
    type: Required[Literal["image_url"]]


ChatCompletionContentPartParam: TypeAlias = Union[
    "ChatCompletionContentPartTextParam",
    "ChatCompletionContentPartImageParam",
]


class ChatCompletionUserMessageParam(TypedDict, total=False):
    content: Required[str | Iterable[ChatCompletionContentPartParam]]
    role: Required[Literal["user"]]
    name: str


class FunctionCall(TypedDict, total=False):
    arguments: Required[str]
    name: Required[str]


class Function(TypedDict, total=False):
    arguments: Required[str]
    name: Required[str]


class ChatCompletionToolMessageParam(TypedDict, total=False):
    content: Required[str | Iterable[ChatCompletionContentPartParam]]
    role: Required[Literal["tool"]]
    tool_call_id: Required[str]


# ast-grep-ignore: no-long-names
class ChatCompletionFunctionMessageParam(TypedDict, total=False):
    content: Required[str | Iterable[ChatCompletionContentPartParam]]
    name: Required[str]
    role: Required[Literal["function"]]


# ast-grep-ignore: no-long-names
class ChatCompletionMessageToolCallParam(TypedDict, total=False):
    id: Required[str]
    function: Required[Function]
    type: Required[Literal["function"]]


# ast-grep-ignore: no-long-names
class ChatCompletionAssistantMessageParam(TypedDict, total=False):
    role: Required[Literal["assistant"]]
    content: str | None
    function_call: FunctionCall
    name: str
    tool_calls: Iterable["ChatCompletionMessageToolCallParam"]


ChatCompletionMessageParam: TypeAlias = Union[
    "ChatCompletionSystemMessageParam",
    "ChatCompletionUserMessageParam",
    "ChatCompletionAssistantMessageParam",
    "ChatCompletionFunctionMessageParam",
    "ChatCompletionToolMessageParam",
]


class CompletionRequest(ResponseObject):
    model: str
    messages: list[ChatCompletionMessageParam]
    timeout: float | int | None
    temperature: float | None
    top_p: float | None
    n: int | None
    stream: bool | None
    stop: dict[str, JsonValue] | None
    max_tokens: int | None
    presence_penalty: float | None
    frequency_penalty: float | None
    logit_bias: dict[str, JsonValue] | None
    user: str | None
    response_format: dict[str, JsonValue] | None
    seed: int | None
    tools: list[str] | None
    tool_choice: str | None
    logprobs: bool | None
    top_logprobs: int | None
    deployment_id: str | None
    functions: list[str] | None
    function_call: str | None
    base_url: str | None
    api_version: str | None
    api_key: str | None
    model_list: list[str] | None
    defaults: ClassVar[dict[str, StoredValue]] = {"messages": []}
