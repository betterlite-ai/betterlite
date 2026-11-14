from collections.abc import Iterator
from typing import Protocol, TypeAlias, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from anthropic_completion.shapes import ChunkView

Messages: TypeAlias = list[AllMessageValues]


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


class EnvArgs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: Messages
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    api_key: str | None
    api_base: str | None


class RequestArgs(TypedDict):
    model: str
    messages: Messages
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: Messages
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None
    api_key: str | None
    json_mode: bool | None


class ChunkParser(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ChunkView: ...


# LiteLLM leaves its dicts bare, so this typed view is the public entry to the text config
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
    ) -> BaseLLMException: ...

    def get_model_response_iterator(
        self, streaming_response: Iterator[str], *, sync_stream: bool, json_mode: bool | None
    ) -> ChunkParser: ...
