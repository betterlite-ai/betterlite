import asyncio
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.caching.caching import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import dynamic_rate_limiter_v3
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitDescriptorRateLimitObject as RateLimitObject,
)
from litellm.router import Router
from litellm.types.router import ModelGroupInfo
from litellm.types.utils import PriorityReservationDict
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_ENCODABLE = dynamic_rate_limiter_v3._is_latin1_encodable  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/dynamic_rate_limiter_v3.py#L62


class ReadJson(TypedDict):
    key: str
    local_only: bool
    ttl: int


class RateLimitJson(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorJson(TypedDict):
    key: str
    value: str
    rate_limit: RateLimitJson | None


class AllocationJson(TypedDict):
    weight: float
    key: str


class Latin1Call(TypedDict):
    function: Literal["latin1"]
    values: list[str]


class AllocationCall(TypedDict):
    function: Literal["allocation"]
    model: str
    rpm: int | None
    tpm: int | None
    window_size: int
    priority_reservation: dict[str, float | PriorityReservationDict] | None
    team_metadata: dict[str, JsonValue] | None
    key_metadata: dict[str, JsonValue]


class SaturationCall(TypedDict):
    function: Literal["saturation"]
    model: str
    rpm: int | None
    tpm: int | None
    cache: dict[str, int | str]


Input: TypeAlias = Latin1Call | AllocationCall | SaturationCall


class Latin1Answer(TypedDict):
    encodable: list[bool]


class AllocationAnswer(TypedDict):
    weights: dict[str, float]
    allocation: AllocationJson
    descriptors: list[DescriptorJson]
    tracking: DescriptorJson


class SaturationAnswer(TypedDict):
    saturation: float
    reads: list[ReadJson]


class ModelInfoPayload(TypedDict):
    model_group: str
    providers: list[str]
    rpm: int | None
    tpm: int | None


class RecordingCache(DualCache):
    def __init__(self, values: dict[str, int | str]) -> None:
        super().__init__()
        self.values = values
        self.reads: list[ReadJson] = []

    @override
    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: Span | None = None,
        local_only: bool = False,
        **kwargs: int,
    ) -> int | str | None:
        self.reads.append(ReadJson(key=key, local_only=local_only, ttl=kwargs["ttl"]))
        return self.values.get(key)


class FixedRouter(Router):
    def __init__(self, info: ModelGroupInfo) -> None:
        self.model_group_info = info

    @override
    def get_model_group_info(self, model_group: str) -> ModelGroupInfo | None:
        del model_group
        return self.model_group_info


_HANDLER_CLASS = dynamic_rate_limiter_v3._PROXY_DynamicRateLimitHandlerV3  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/dynamic_rate_limiter_v3.py#L66


class SatHandler(_HANDLER_CLASS):
    def priority_of(self, auth: UserAPIKeyAuth) -> str | None:
        return self._get_priority_from_user_api_key_dict(auth)

    def weights(self, info: ModelGroupInfo) -> dict[str, float]:
        return self._normalize_priority_weights(info)

    def allocation(
        self,
        model: str,
        priority: str | None,
        weights: dict[str, float],
        info: ModelGroupInfo,
    ) -> tuple[float, str]:
        return self._get_priority_allocation(
            model=model, priority=priority, normalized_weights=weights, model_info=info
        )

    def priority_descriptors(
        self, model: str, auth: UserAPIKeyAuth, priority: str | None
    ) -> list[RateLimitDescriptor]:
        return self._create_priority_based_descriptors(
            model=model, user_api_key_dict=auth, priority=priority
        )

    def tracking(self, model: str, info: ModelGroupInfo) -> RateLimitDescriptor:
        return self._create_model_tracking_descriptor(
            model=model, model_group_info=info, high_limit_multiplier=1
        )

    async def saturation(self, model: str, info: ModelGroupInfo) -> float:
        return await self._check_model_saturation(model, info)


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


def latin1_answer(call: Latin1Call) -> Latin1Answer:
    flags = [_ENCODABLE(value) for value in call["values"]]
    return Latin1Answer(encodable=flags)


def allocation_answer(call: AllocationCall) -> AllocationAnswer:
    info = model_group_info(call["model"], call["rpm"], call["tpm"])
    handler = SatHandler(internal_usage_cache=RecordingCache({}))
    handler.v3_limiter.window_size = call["window_size"]
    handler.update_variables(FixedRouter(info))
    litellm.priority_reservation = call["priority_reservation"]
    auth = UserAPIKeyAuth(metadata=call["key_metadata"], team_metadata=call["team_metadata"])
    priority = handler.priority_of(auth)
    weights = handler.weights(info)
    weight, pool = handler.allocation(call["model"], priority, weights, info)
    descriptors = handler.priority_descriptors(call["model"], auth, priority)
    tracking = handler.tracking(call["model"], info)
    return AllocationAnswer(
        weights=weights,
        allocation=AllocationJson(weight=weight, key=pool),
        descriptors=[descriptor_json(descriptor) for descriptor in descriptors],
        tracking=descriptor_json(tracking),
    )


async def saturation_answer(call: SaturationCall) -> SaturationAnswer:
    info = model_group_info(call["model"], call["rpm"], call["tpm"])
    cache = RecordingCache(call["cache"])
    handler = SatHandler(internal_usage_cache=cache)
    saturation = await handler.saturation(call["model"], info)
    return SaturationAnswer(saturation=saturation, reads=cache.reads)


def saturation_run(call: SaturationCall) -> SaturationAnswer:
    with asyncio.Runner() as runner:
        return runner.run(saturation_answer(call))


def run(input_value: Input) -> Latin1Answer | AllocationAnswer | SaturationAnswer:
    if input_value["function"] == "latin1":
        return latin1_answer(input_value)
    if input_value["function"] == "allocation":
        return allocation_answer(input_value)
    return saturation_run(input_value)
