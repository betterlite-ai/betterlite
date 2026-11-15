from collections.abc import Iterator
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import ModelResponse
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


class BodyView(TypedDict):
    prompt: str


class ChoiceMessage(TypedDict):
    content: str | None


class ChoiceView(TypedDict):
    finish_reason: str | None
    message: ChoiceMessage


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseView(TypedDict):
    id: str
    model: str
    created: int
    choices: list[ChoiceView]
    usage: UsageView


class ChunkView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    index: int


class EnvArgs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[Turn]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    api_key: str | None


class RequestArgs(TypedDict):
    model: str
    messages: list[Turn]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: list[Turn]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None


class ErrorEnvelope(Protocol):
    status_code: int
    message: str
    request: httpx.Request
    response: httpx.Response
    headers: dict[str, str] | None


class ChunkParser(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ChunkView: ...


class Registry(Protocol):
    def register_prompt_template(
        self,
        model: str,
        roles: dict[str, dict[str, str]],
        initial_prompt_value: str,
        final_prompt_value: str,
    ) -> JsonValue: ...


# LiteLLM leaves its parameters untyped, so this typed view is the public entry to the text config
class TextConfig(Protocol):
    def validate_environment(self, **kwargs: Unpack[EnvArgs]) -> dict[str, str]: ...

    def transform_request(self, **kwargs: Unpack[RequestArgs]) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_response(self, **kwargs: Unpack[ResponseArgs]) -> ModelResponse: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> ErrorEnvelope: ...

    def get_model_response_iterator(
        self, streaming_response: Iterator[str], *, sync_stream: bool, json_mode: bool | None
    ) -> ChunkParser: ...
