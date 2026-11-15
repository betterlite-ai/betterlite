from typing import Literal, Required, TypeAlias, TypedDict

from litellm.types.llms.openai import ResponseAPIUsage
from litellm.types.rerank import RerankResponse
from litellm.types.utils import EmbeddingResponse, Usage
from pydantic import JsonValue


class WindowInput(TypedDict):
    counter_key: str
    window_start: str
    backend: Literal["redis", "local"]


class ModelInput(TypedDict):
    requested: str
    group: str


class StashInput(TypedDict, total=False):
    owner_call_id: str
    reserved_tokens: int
    reserved_model: ModelInput
    reserved_scopes: list[list[str]]
    itpm_reserved_tokens: int
    itpm_reserved_scopes: list[list[str]]
    itpm_reserved_windows: list[WindowInput]
    otpm_reserved_tokens: int
    otpm_reserved_scopes: list[list[str]]
    otpm_reserved_windows: list[WindowInput]
    reservation_released: bool
    tpm_limited_tags: list[str]


class ResponseInput(TypedDict, total=False):
    kind: Literal[
        "missing", "usage", "response_api", "mapping", "with_usage", "rerank", "unmodelled"
    ]
    usage: dict[str, JsonValue]
    response_api_usage: dict[str, JsonValue]
    rerank_tokens: dict[str, int]
    billed_units_total_tokens: int


class IoTokenCase(TypedDict, total=False):
    function: Required[Literal["io_token_ops"]]
    call_id: str
    stash: StashInput
    response: ResponseInput
    combined_usage: dict[str, JsonValue]
    window_size: int


class ScopeTargetsCase(TypedDict, total=False):
    function: Required[Literal["scope_targets"]]
    logging: dict[str, JsonValue]
    request_user: str
    model_group: str
    tags: list[str]


class SuccessCase(TypedDict, total=False):
    function: Required[Literal["success_ops"]]
    call_id: str
    stash: StashInput
    response: ResponseInput
    combined_usage: dict[str, JsonValue]
    rate_limit_type: Literal["input", "output", "total"]
    logging: dict[str, JsonValue]
    request_metadata: dict[str, JsonValue]
    request_user: str
    model_group: str
    groups: dict[str, str]
    window_size: int


Input: TypeAlias = IoTokenCase | ScopeTargetsCase | SuccessCase


class CallbackArguments(TypedDict, total=False):
    litellm_call_id: str
    combined_usage_object: Usage
    litellm_params: dict[str, JsonValue]
    standard_logging_object: dict[str, JsonValue]
    user: str


ResponsePayload: TypeAlias = (
    Usage | ResponseAPIUsage | EmbeddingResponse | RerankResponse | dict[str, JsonValue] | None
)
