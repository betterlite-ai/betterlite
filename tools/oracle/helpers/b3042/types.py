from collections.abc import Sequence
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
)
from litellm.types.caching import RedisPipelineIncrementOperation
from pydantic import BaseModel, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class AuthFields(TypedDict, total=False):
    project_id: str
    project_metadata: dict[str, JsonValue]


class WindowIdentity(TypedDict):
    counter_key: str
    window_start: str
    backend: Literal["redis", "local"]


class CaseBase(TypedDict):
    window_size: int
    now: int
    groups: dict[str, str]
    cache: dict[str, int | float | str]


class ReserveTpmInput(CaseBase):
    function: Literal["reserve_tpm_tokens"]
    descriptors: list[RateLimitDescriptor]
    estimated_tokens: int


class RefundInput(CaseBase):
    function: Literal["refund_reserved_tokens"]
    scopes: list[list[str]]
    amount: int
    reservation_windows: list[WindowIdentity]


class ReserveIoInput(CaseBase):
    function: Literal["reserve_io_tokens"]
    descriptors: list[RateLimitDescriptor]
    estimated_input_tokens: int
    estimated_output_tokens: int


class FrameInput(CaseBase):
    function: Literal["enforce_project_io_token_quota_for_frame"]
    auth: AuthFields | None
    requested_model: str | None
    estimated_input_tokens: int
    estimated_output_tokens: int


Input: TypeAlias = ReserveTpmInput | RefundInput | ReserveIoInput | FrameInput


class RaisedFields(BaseModel):
    status_code: int
    detail: str
    headers: dict[str, str] | None
    rate_limit_type: str | None
    category: str


class UsageCache(Protocol):
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> int | float | str | None: ...

    async def async_set_cache(
        self,
        key: str,
        value: int | float | str,
        ttl: int,
        *,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None: ...

    async def async_increment_cache(
        self, key: str, value: float, *, litellm_parent_otel_span: JsonValue, ttl: int
    ) -> None: ...


class IncrementPipeline(Protocol):
    async def async_increment_cache(
        self, key: str, value: float, *, litellm_parent_otel_span: JsonValue, ttl: int
    ) -> None: ...

    async def async_increment_cache_pipeline(
        self,
        increment_list: list[RedisPipelineIncrementOperation],
        *,
        litellm_parent_otel_span: JsonValue,
    ) -> None: ...


class Operations(Protocol):
    async def reserve_tpm_tokens(
        self, descriptors: list[RateLimitDescriptor], estimated_tokens: int
    ) -> RateLimitResponse: ...

    async def reserve_io_tokens(
        self,
        descriptors: list[RateLimitDescriptor],
        estimated_input_tokens: int,
        estimated_output_tokens: int,
    ) -> tuple[RateLimitResponse, int, int]: ...

    async def _refund_reserved_tokens(
        self,
        scopes: Sequence[tuple[str, str]],
        amount: int,
        reservation_windows: frozenset[tuple[str, str, Literal["redis", "local"]]],
    ) -> None: ...

    async def enforce_project_io_token_quota_for_frame(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth | None,
        requested_model: str | None,
        estimated_input_tokens: int,
        estimated_output_tokens: int,
    ) -> None: ...

    async def refund_reserved(
        self,
        scopes: list[list[str]],
        amount: int,
        windows: list[WindowIdentity],
    ) -> None:
        targets: list[tuple[str, str]] = [(scope[0], scope[1]) for scope in scopes]
        reservation_windows: frozenset[tuple[str, str, Literal["redis", "local"]]] = frozenset(
            (window["counter_key"], window["window_start"], window["backend"])
            for window in windows
        )
        await self._refund_reserved_tokens(targets, amount, reservation_windows)
