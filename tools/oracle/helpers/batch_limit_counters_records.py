from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from litellm.proxy.hooks.batch_enqueued_tokens import (
    BatchEnqueuedTokenReservation,
    BatchEnqueuedTokenScope,
)
from litellm.proxy.hooks.batch_rate_limiter import BatchFileUsage
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitStatus,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptorRateLimitObject as RateLimitFields,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    ReservationAwareIncrementOperation as RefundOperation,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
STATUS: Final = TypeAdapter(RateLimitStatus)
BATCH_USAGE: Final = TypeAdapter(BatchFileUsage)
Function: TypeAlias = Literal[
    "reserve_batch_enqueued_tokens",
    "raise_enqueued_limit_error",
    "raise_rate_limit_error",
    "check_and_increment_batch_counters",
    "read_tpd_window_start",
    "build_tpd_refund_ops",
]
LimitType: TypeAlias = Literal["requests", "tokens", "max_parallel_requests"]
Backend: TypeAlias = Literal["redis", "local"]


class ScopeSpec(TypedDict):
    key: Literal["api_key", "team"]
    value: str
    limit: int


class UsageSpec(TypedDict):
    total_tokens: int
    request_count: int
    output_tokens: int
    per_model_usage: dict[str, dict[str, int]]


class StatusSpec(TypedDict):
    code: str
    current_limit: int
    limit_remaining: int
    rate_limit_type: LimitType
    descriptor_key: str
    descriptor_value: NotRequired[str | None]


class WindowSpec(TypedDict):
    counter_key: str
    window_start: str
    backend: Backend


class ResponseSpec(TypedDict):
    overall_code: str
    statuses: list[StatusSpec]
    reservation_windows: NotRequired[list[WindowSpec]]


class OverLimitSpec(TypedDict):
    scope: ScopeSpec
    enqueued: int


class ReplySpec(TypedDict):
    kind: Literal["reservation", "over_limit"]
    backend: NotRequired[Literal["redis", "memory"]]
    owner: NotRequired[str]
    scope: NotRequired[ScopeSpec]
    enqueued: NotRequired[int]


class ProjectScript(TypedDict):
    descriptors: list[RateLimitDescriptor]
    increments: list[dict[Literal["requests", "tokens"], int]]


class LimiterScript(TypedDict):
    base: list[RateLimitDescriptor]
    project: ProjectScript
    responses: list[ResponseSpec]


class StoreScript(TypedDict):
    replies: list[ReplySpec]


class Call(TypedDict):
    function: Function
    data: dict[str, JsonValue]
    batch_usage: UsageSpec
    scopes: list[ScopeSpec]
    over_limit: OverLimitSpec
    status: StatusSpec
    descriptors: list[RateLimitDescriptor]
    limit_type: str
    requested_model: str | None
    window_start: int | None
    tokens: int
    reservation_windows: NotRequired[list[WindowSpec]]


class ScopeView(TypedDict):
    key: Literal["api_key", "team"]
    value: str
    limit: int


class RateLimitView(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorView(TypedDict):
    key: str
    value: str
    rate_limit: RateLimitView | None


class IncrementView(TypedDict):
    requests: int
    tokens: int


class LimiterCallView(TypedDict):
    descriptors: list[DescriptorView]
    increments: list[IncrementView]


class StoreCallView(TypedDict):
    tokens: int
    scopes: list[ScopeView]


class OperationView(TypedDict):
    key: str
    increment_value: float
    ttl: int | None
    window_key: str | None
    expected_window_start: str | None
    reservation_backend: Backend | None


class ReservationView(TypedDict):
    tokens: int
    scopes: list[ScopeView]
    backend: Literal["redis", "memory"]
    owner: str


class ErrorView(TypedDict):
    status: int
    headers: dict[str, str]
    category: str
    rate_limit_type: str | None
    model: str
    llm_provider: str


class Effects(TypedDict):
    store_calls: list[StoreCallView]
    limiter_calls: list[LimiterCallView]
    base_calls: int
    project_calls: int
    cache_reads: list[str]
    reservation: ReservationView | None
    refund_ops: list[OperationView]


class ErrorFields(Protocol):
    status_code: int
    headers: dict[str, str] | None
    category: str
    rate_limit_type: str | None
    model: str
    llm_provider: str


def status_of(raw: StatusSpec) -> RateLimitStatus:
    fields: dict[str, JsonValue] = {
        "code": raw["code"],
        "current_limit": raw["current_limit"],
        "limit_remaining": raw["limit_remaining"],
        "rate_limit_type": raw["rate_limit_type"],
        "descriptor_key": raw["descriptor_key"],
    }
    value = raw.get("descriptor_value")
    if value is not None:
        fields["descriptor_value"] = value
    return STATUS.validate_python(fields)


def windows_of(held: list[WindowSpec] | None) -> frozenset[tuple[str, str, Backend]]:
    listed: list[WindowSpec] = [] if held is None else held
    return frozenset(
        (item["counter_key"], item["window_start"], item["backend"]) for item in listed
    )


def response_of(raw: ResponseSpec) -> RateLimitResponse:
    return {
        "overall_code": raw["overall_code"],
        "statuses": [status_of(status) for status in raw["statuses"]],
        "reservation_windows": windows_of(raw.get("reservation_windows")),
    }


def descriptor_view(descriptor: RateLimitDescriptor) -> DescriptorView:
    rate_limit: RateLimitFields | None = descriptor.get("rate_limit")
    if rate_limit is None:
        return {"key": descriptor["key"], "value": descriptor["value"], "rate_limit": None}
    return {
        "key": descriptor["key"],
        "value": descriptor["value"],
        "rate_limit": {
            "requests_per_unit": rate_limit.get("requests_per_unit"),
            "tokens_per_unit": rate_limit.get("tokens_per_unit"),
            "max_parallel_requests": rate_limit.get("max_parallel_requests"),
            "window_size": rate_limit.get("window_size"),
        },
    }


def operation_view(operation: RefundOperation) -> OperationView:
    return {
        "key": operation["key"],
        "increment_value": operation["increment_value"],
        "ttl": operation.get("ttl"),
        "window_key": operation.get("window_key"),
        "expected_window_start": operation.get("expected_window_start"),
        "reservation_backend": operation.get("reservation_backend"),
    }


def scopes_of(scopes: tuple[BatchEnqueuedTokenScope, ...]) -> list[ScopeView]:
    return [
        {"key": scope.key, "value": scope.value, "limit": scope.limit} for scope in scopes
    ]


def reservation_view(reservation: BatchEnqueuedTokenReservation) -> ReservationView:
    return {
        "tokens": reservation.tokens,
        "scopes": scopes_of(reservation.scopes),
        "backend": reservation.backend,
        "owner": reservation.owner,
    }


def error_view(error: ProxyRateLimitError) -> ErrorView:
    fields = cast(ErrorFields, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L192
    held = fields.headers
    return {
        "status": fields.status_code,
        "headers": {} if held is None else held,
        "category": fields.category,
        "rate_limit_type": fields.rate_limit_type,
        "model": fields.model,
        "llm_provider": fields.llm_provider,
    }
