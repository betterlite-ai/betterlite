from typing import Literal, Required, TypeAlias, TypedDict

from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitStatus,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ReservationAwareIncrementOperation as PipelineOperation,
)
from litellm.types.utils import Usage
from pydantic import JsonValue

Function: TypeAlias = Literal[
    "header_merge",
    "logging_hook",
    "success_event",
    "failure_event",
    "disconnect",
    "success_hook",
    "failure_hook",
]


class HiddenParams(TypedDict):
    additional_headers: dict[str, JsonValue]


class WindowInput(TypedDict):
    counter_key: str
    window_start: str
    backend: Literal["redis", "local"]


class SlotInput(TypedDict):
    slot_id: str
    counter_keys: list[str]


class ResponseInput(TypedDict, total=False):
    id: str
    status: str
    object: Literal["batch"]
    _hidden_params: HiddenParams


class StashInput(TypedDict, total=False):
    owner_call_id: str
    parallel_slot: SlotInput
    statuses: list[RateLimitStatus]
    reserved_tokens: int
    reserved_scopes: list[list[str]]
    itpm_reserved_tokens: int
    itpm_reserved_scopes: list[list[str]]
    itpm_reserved_windows: list[WindowInput]
    otpm_reserved_tokens: int
    otpm_reserved_scopes: list[list[str]]
    otpm_reserved_windows: list[WindowInput]
    batch_tokens: int
    batch_tpd_ops: list[PipelineOperation]
    reservation_released: bool


class Case(TypedDict, total=False):
    function: Required[Function]
    call_id: str
    stash: StashInput
    headers: dict[str, JsonValue]
    usage: dict[str, JsonValue]
    logging: dict[str, JsonValue]
    response: ResponseInput
    pop_tokens: int
    runs: int
    seed: dict[str, JsonValue]


class LiteLLMParams(TypedDict):
    metadata: dict[str, JsonValue]


class LoggingPayload(TypedDict, total=False):
    metadata: dict[str, JsonValue]
    hidden_params: HiddenParams


class CallbackArguments(TypedDict, total=False):
    litellm_call_id: str
    combined_usage_object: Usage
    litellm_params: LiteLLMParams
    standard_logging_object: LoggingPayload


Input: TypeAlias = Case
