from collections.abc import Callable
from dataclasses import dataclass
from typing import Literal, NotRequired, Protocol, TypedDict, Unpack

from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

Action = Literal[
    "messages", "legacy", "count", "batch_create", "batch_retrieve", "batch_results",
    "upload", "retrieve", "list", "delete", "content", "config_url",
]


class Input(TypedDict):
    action: Action
    model: str
    provider: str
    api_base: str | None
    api_key: str | None
    global_base: str | None
    anthropic_key: str | None
    global_key: str | None
    environment: dict[str, str]
    disable_suffix: bool
    config_present: bool
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    custom_prompt_dict: dict[str, JsonValue]
    global_prompt_dict: dict[str, JsonValue]
    stream: bool
    acompletion: bool
    client: str | None
    shared_session: str | None
    encoding: str
    expected_key: str | None
    failure: int | None
    response: str
    id: str
    purpose: str | None
    file: str
    system: JsonValue
    tools: list[dict[str, JsonValue]] | None


class Recorded(TypedDict):
    method: str
    url: str
    query: list[list[str]]
    headers: dict[str, str]
    body: JsonValue


class Observation(TypedDict):
    result: JsonValue
    post_call: bool
    post_matches: bool
    request: Recorded | None


class Case(TypedDict):
    inputs: list[Input]
    expected: list[Observation]


class PostArguments(TypedDict):
    input: list[dict[str, JsonValue]]
    api_key: str | None
    original_response: ModelResponse


class PostRecorder(Protocol):
    def post_call(self, **kwargs: Unpack[PostArguments]) -> None: ...


class Arguments(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str
    api_key: str | None
    custom_llm_provider: str
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    encoding: str
    acompletion: bool
    model_response: ModelResponse
    logging_obj: PostRecorder
    timeout: int
    custom_prompt_dict: NotRequired[dict[str, JsonValue]]
    stream: NotRequired[bool]
    client: NotRequired[HTTPHandler | None]
    shared_session: NotRequired[str | None]
    print_verbose: NotRequired[Callable[[str], None]]
    logger_fn: NotRequired[None]


@dataclass
class Context:
    acompletion: bool
    api_base: str | None
    api_key: str | None
    custom_prompt_dict: dict[str, JsonValue]
    headers: dict[str, str]
    litellm_params: dict[str, JsonValue]
    logging: PostRecorder
    messages: list[dict[str, JsonValue]]
    model: str
    model_response: ModelResponse
    optional_params: dict[str, JsonValue]
    shared_session: str | None
    stream: bool
    timeout: int
    client: HTTPHandler | None
    custom_llm_provider: str
    logger_fn: None
