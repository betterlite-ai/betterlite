import asyncio
from collections.abc import Sequence
from typing import Final, Literal, TypedDict

import litellm
from litellm.caching.dual_cache import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from litellm.proxy.hooks import dynamic_rate_limiter_v3, parallel_request_limiter_v3
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitStatus,
    get_request_stash_for_call,
)
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptorRateLimitObject as RateLimitObject,
)
from litellm.router import Router
from litellm.types.router import ModelGroupInfo
from litellm.types.utils import PriorityReservationDict, PriorityReservationSettings
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_HANDLER_CLASS = dynamic_rate_limiter_v3._PROXY_DynamicRateLimitHandlerV3  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/dynamic_rate_limiter_v3.py#L66
_STUB_BASE = parallel_request_limiter_v3._PROXY_MaxParallelRequestsHandler_v3  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L751
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class AdmissionInput(TypedDict):
    model: str
    rpm: int | None
    tpm: int | None
    window_size: int
    priority: str | None
    priority_reservation: dict[str, float | PriorityReservationDict] | None
    default_priority: float
    saturation_threshold: float
    saturation: float
    atomic: RateLimitResponse
    tracking: RateLimitResponse


class RateLimitJson(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorJson(TypedDict):
    key: str
    value: str
    rate_limit: RateLimitJson | None


class IncrementJson(TypedDict):
    requests: int
    tokens: int


class AtomicCallJson(TypedDict):
    descriptors: list[DescriptorJson]
    increments: list[IncrementJson]


class TrackingCallJson(TypedDict):
    descriptors: list[DescriptorJson]
    read_only: bool


class StatusJson(TypedDict):
    code: str
    current_limit: int
    limit_remaining: int
    rate_limit_type: str
    descriptor_key: str
    descriptor_value: str | None


class StashJson(TypedDict):
    overall_code: str
    statuses: list[StatusJson]


class ErrorJson(TypedDict):
    status: int
    detail: JsonValue
    headers: dict[str, str]


class CheckCall(TypedDict):
    model: str
    model_group_info: ModelGroupInfo
    user_api_key_dict: UserAPIKeyAuth
    priority: str | None
    saturation: float


class ModelInfoPayload(TypedDict):
    model_group: str
    providers: list[str]
    rpm: int | None
    tpm: int | None


class CheckHandler(_HANDLER_CLASS):
    async def check(self, call: CheckCall) -> None:
        await self._check_rate_limits(
            model=call["model"],
            model_group_info=call["model_group_info"],
            user_api_key_dict=call["user_api_key_dict"],
            priority=call["priority"],
            saturation=call["saturation"],
        )


class FixedRouter(Router):
    def __init__(self, info: ModelGroupInfo) -> None:
        self.model_group_info = info

    @override
    def get_model_group_info(self, model_group: str) -> ModelGroupInfo | None:
        del model_group
        return self.model_group_info


class StubLimiter(_STUB_BASE):
    def __init__(self, input_value: AdmissionInput) -> None:
        self.window_size = input_value["window_size"]
        self.atomic = input_value["atomic"]
        self.tracking = input_value["tracking"]
        self.atomic_calls: list[AtomicCallJson] = []
        self.tracking_calls: list[TrackingCallJson] = []

    @override
    async def atomic_check_and_increment_by_n(
        self,
        descriptors: list[RateLimitDescriptor],
        increments: list[dict[Literal["requests", "tokens"], int]],
        parent_otel_span: Span | None = None,
    ) -> RateLimitResponse:
        del parent_otel_span
        if len(increments) != len(descriptors):
            raise ValueError("the handler must send one increment per descriptor")
        self.atomic_calls.append(
            AtomicCallJson(
                descriptors=[descriptor_json(descriptor) for descriptor in descriptors],
                increments=[
                    IncrementJson(requests=increment["requests"], tokens=increment["tokens"])
                    for increment in increments
                ],
            )
        )
        return self.atomic

    @override
    async def should_rate_limit(
        self,
        descriptors: Sequence[RateLimitDescriptor],
        parent_otel_span: Span | None = None,
        read_only: bool = False,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L1528-L1535
        skip_tpm_check: bool = False,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/parallel_request_limiter_v3.py#L1528-L1535
        parallel_slot_id: str | None = None,
    ) -> RateLimitResponse:
        del parent_otel_span, skip_tpm_check, parallel_slot_id
        self.tracking_calls.append(
            TrackingCallJson(
                descriptors=[descriptor_json(descriptor) for descriptor in descriptors],
                read_only=read_only,
            )
        )
        return self.tracking


def model_group_info(model: str, rpm: int | None, tpm: int | None) -> ModelGroupInfo:
    payload = ModelInfoPayload(model_group=model, providers=[], rpm=rpm, tpm=tpm)
    return TypeAdapter(ModelGroupInfo).validate_python(payload)


def rate_limit_json(rate_limit: RateLimitObject) -> RateLimitJson:
    canonical = RateLimitJson(
        requests_per_unit=None,
        tokens_per_unit=None,
        max_parallel_requests=None,
        window_size=None,
    )
    if "requests_per_unit" in rate_limit:
        canonical["requests_per_unit"] = rate_limit["requests_per_unit"]
    if "tokens_per_unit" in rate_limit:
        canonical["tokens_per_unit"] = rate_limit["tokens_per_unit"]
    if "max_parallel_requests" in rate_limit:
        canonical["max_parallel_requests"] = rate_limit["max_parallel_requests"]
    if "window_size" in rate_limit:
        canonical["window_size"] = rate_limit["window_size"]
    return canonical


def descriptor_json(descriptor: RateLimitDescriptor) -> DescriptorJson:
    rate_limit = descriptor["rate_limit"]
    canonical: RateLimitJson | None = None
    if rate_limit is not None:
        canonical = rate_limit_json(rate_limit)
    return DescriptorJson(key=descriptor["key"], value=descriptor["value"], rate_limit=canonical)


def status_json(status: RateLimitStatus) -> StatusJson:
    return StatusJson(
        code=status["code"],
        current_limit=status["current_limit"],
        limit_remaining=status["limit_remaining"],
        rate_limit_type=status["rate_limit_type"],
        descriptor_key=status["descriptor_key"],
        descriptor_value=status.get("descriptor_value"),
    )


def response_json(response: RateLimitResponse) -> StashJson:
    return StashJson(
        overall_code=response["overall_code"],
        statuses=[status_json(status) for status in response["statuses"]],
    )


def error_json(rejected: ProxyRateLimitError) -> ErrorJson:
    headers: dict[str, str] = {}
    if rejected.headers is not None:
        headers = {name: str(value) for name, value in rejected.headers.items()}
    return ErrorJson(
        status=rejected.status_code,
        detail=JSON.validate_python(rejected.detail),  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L146
        headers=headers,
    )


async def admission_answer(input_value: AdmissionInput) -> dict[str, JsonValue]:
    info = model_group_info(input_value["model"], input_value["rpm"], input_value["tpm"])
    litellm.priority_reservation = input_value["priority_reservation"]
    litellm.priority_reservation_settings = PriorityReservationSettings(
        default_priority=input_value["default_priority"],
        saturation_threshold=input_value["saturation_threshold"],
        saturation_check_cache_ttl=60,
    )
    handler = CheckHandler(internal_usage_cache=DualCache())
    limiter = StubLimiter(input_value)
    handler.v3_limiter = limiter
    handler.update_variables(FixedRouter(info))
    key_metadata: dict[str, str] = {}
    auth = UserAPIKeyAuth(metadata=key_metadata, team_metadata=None)
    error: ErrorJson | None = None
    try:
        await handler.check(
            CheckCall(
                model=input_value["model"],
                model_group_info=info,
                user_api_key_dict=auth,
                priority=input_value["priority"],
                saturation=input_value["saturation"],
            )
        )
    except ProxyRateLimitError as rejected:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L101
        error = error_json(rejected)
    stash = get_request_stash_for_call(None)
    stash_value: StashJson | None = None
    if stash is not None and stash.rate_limit_response is not None:
        stash_value = response_json(stash.rate_limit_response)
    return {
        "atomic_calls": JSON.validate_python(limiter.atomic_calls),
        "tracking_calls": JSON.validate_python(limiter.tracking_calls),
        "stash": JSON.validate_python(stash_value),
        "error": JSON.validate_python(error),
    }


def run(input_value: AdmissionInput) -> dict[str, JsonValue]:
    with asyncio.Runner() as runner:
        return runner.run(admission_answer(input_value))
