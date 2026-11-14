from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue


class Turn(TypedDict):
    role: str
    content: str


class CustomPrompt(TypedDict):
    roles: dict[str, dict[str, str]]
    initial_prompt_value: str
    final_prompt_value: str


class SelectInput(TypedDict):
    function: Literal["select"]
    models: list[str]


class PromptCase(TypedDict):
    model: str
    messages: list[Turn]
    custom_prompt: CustomPrompt | None


class PromptInput(TypedDict):
    function: Literal["prompt"]
    cases: list[PromptCase]


class AuthCase(TypedDict):
    api_key: str | None
    headers: dict[str, str]


class AuthInput(TypedDict):
    function: Literal["auth"]
    cases: list[AuthCase]


class Constructor(TypedDict):
    max_tokens_to_sample: int | None
    stop_sequences: list[str] | None
    top_k: int | None
    metadata: dict[str, JsonValue] | None


class OptionCase(TypedDict):
    model: str
    messages: list[Turn]
    non_default: dict[str, JsonValue]
    existing: dict[str, JsonValue]


class OptionsInput(TypedDict):
    function: Literal["options"]
    constructor: Constructor
    cases: list[OptionCase]


class ReplyCase(TypedDict):
    model: str
    messages: list[Turn]
    status: int
    body: str
    initial_content: str


class ReplyInput(TypedDict):
    function: Literal["reply"]
    cases: list[ReplyCase]


class ChunkInput(TypedDict):
    function: Literal["chunk"]
    chunks: list[dict[str, JsonValue]]


class ErrorCase(TypedDict):
    message: str
    status_code: int
    headers: dict[str, str]


class ErrorInput(TypedDict):
    function: Literal["error"]
    cases: list[ErrorCase]


Input: TypeAlias = (
    SelectInput | PromptInput | AuthInput | OptionsInput | ReplyInput | ChunkInput | ErrorInput
)


class SelectOutput(TypedDict):
    selected: list[bool]


class PromptOutput(TypedDict):
    prompts: list[str]


class Accepted(TypedDict):
    ok: bool
    headers: dict[str, str]


class Refused(TypedDict):
    ok: bool
    message: str


class AuthOutput(TypedDict):
    results: list[Accepted | Refused]


class OptionView(TypedDict):
    mapped: dict[str, JsonValue]
    body: dict[str, JsonValue]


class OptionsOutput(TypedDict):
    supported: list[str]
    cases: list[OptionView]


class Replied(TypedDict):
    ok: bool
    content: str | None
    finish_reason: str | None
    id: str
    model: str
    created_is_current: bool
    usage: list[int]


class Failed(TypedDict):
    ok: bool
    status: int
    message: str


class ReplyOutput(TypedDict):
    replies: list[Replied | Failed]


class ChunkView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    index: int


class ChunkOutput(TypedDict):
    chunks: list[ChunkView]


class ErrorView(TypedDict):
    status_code: int
    message: str
    method: str
    url: str
    response_status: int
    headers_kept: bool


class ErrorOutput(TypedDict):
    errors: list[ErrorView]


Output: TypeAlias = (
    SelectOutput | PromptOutput | AuthOutput | OptionsOutput | ReplyOutput | ChunkOutput | ErrorOutput
)
