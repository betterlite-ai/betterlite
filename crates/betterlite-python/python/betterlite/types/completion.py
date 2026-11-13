from collections.abc import Iterable
from typing import Literal, Required, TypeAlias, TypedDict

from pydantic import Field, JsonValue
from typing_extensions import override

from betterlite.types.utils.base import InitValue, ResponseObject


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


ChatCompletionContentPartParam: TypeAlias = (
    # ast-grep-ignore: no-long-names
    ChatCompletionContentPartTextParam | ChatCompletionContentPartImageParam
)


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


ChatCompletionMessageParam: TypeAlias = (
    # ast-grep-ignore: no-long-names
    ChatCompletionSystemMessageParam
    | ChatCompletionUserMessageParam
    # ast-grep-ignore: no-long-names
    | ChatCompletionAssistantMessageParam
    # ast-grep-ignore: no-long-names
    | ChatCompletionFunctionMessageParam
    | ChatCompletionToolMessageParam
)


class CompletionRequest(ResponseObject):
    model: str
    messages: list[ChatCompletionMessageParam] = Field(default_factory=list)
    timeout: float | int | None = None
    temperature: float | None = None
    top_p: float | None = None
    n: int | None = None
    stream: bool | None = None
    stop: dict[str, JsonValue] | None = None
    max_tokens: int | None = None
    presence_penalty: float | None = None
    frequency_penalty: float | None = None
    logit_bias: dict[str, JsonValue] | None = None
    user: str | None = None
    response_format: dict[str, JsonValue] | None = None
    seed: int | None = None
    tools: list[str] | None = None
    tool_choice: str | None = None
    logprobs: bool | None = None
    top_logprobs: int | None = None
    deployment_id: str | None = None
    functions: list[str] | None = None
    function_call: str | None = None
    base_url: str | None = None
    api_version: str | None = None
    api_key: str | None = None
    model_list: list[str] | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)
