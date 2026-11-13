from collections.abc import Callable
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.llms.anthropic.common_utils import (
    AnthropicModelInfo,
    create_anthropic_model_list_response,
    process_anthropic_headers,
)
from pydantic import JsonValue as Json

BetaName: TypeAlias = Literal[
    "code_execution",
    "effort",
    "file_id",
    "input_examples",
    "mcp_server",
    "output_config",
    "pdf",
    "programmatic_tool_calling",
    "prompt_caching",
    "skills",
    "thinking_display_updates",
    "tool_changes",
    "tool_search",
]


class Header(TypedDict):
    name: str
    value: str


class ModelRecord(TypedDict):
    id: str
    object: Literal["model"]
    created: int
    owned_by: str
    max_input_tokens: NotRequired[int]
    max_output_tokens: NotRequired[int]


class HeadersCall(TypedDict):
    function: Literal["headers"]
    api_key: str | None
    auth_token: str | None
    anthropic_version: str | None
    api_base: str | None
    use_bearer_for_custom_base: bool
    user_betas: list[str] | None
    features: list[BetaName]
    computer_tool_used: str | None
    is_vertex_request: bool
    web_search_tool_used: bool


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    model: str
    headers: list[Header]
    api_key: str | None
    api_base: str | None
    litellm_params: dict[str, Json]
    messages: list[Json]
    optional_params: dict[str, Json]
    effort_used: bool


class ModelListCall(TypedDict):
    function: Literal["model_list"]
    models: list[ModelRecord]
    display_names: dict[str, str]
    listed_ids: dict[str, str]


class ResponseHeadersCall(TypedDict):
    function: Literal["response_headers"]
    headers: list[Header]


Call: TypeAlias = HeadersCall | EnvironmentCall | ModelListCall | ResponseHeadersCall


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[Call]


GET_HEADERS: Final[Callable[..., dict[str, str]]] = AnthropicModelInfo().get_anthropic_headers
VALIDATE: Final[Callable[..., dict[str, str]]] = AnthropicModelInfo().validate_environment
PROCESS_HEADERS: Final[Callable[..., dict[str, str]]] = process_anthropic_headers
CREATE_MODEL_LIST: Final = create_anthropic_model_list_response
