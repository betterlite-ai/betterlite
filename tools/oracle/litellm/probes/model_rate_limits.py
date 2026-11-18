import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, final
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache, RedisCircuitBreakerOpenError
from litellm.exceptions import RateLimitError
from litellm.router_utils.pre_call_checks import io_token_rate_limit_check as io_check
from litellm.router_utils.pre_call_checks import model_rate_limit_check as model_check
from litellm.router_utils.pre_call_checks.model_rate_limit_check import ModelRateLimitingCheck
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MILLIS_PER_SECOND: Final = 1000
ROUTING_ARGS_TTL: Final = 60
RedisMode = Literal["none", "up", "breaker", "down"]


class DeploymentInput(Strict):
    id: str | None = None
    model: str | None = None
    model_name: str
    tpm: int | None = None
    rpm: int | None = None
    itpm: int | None = None


class Counters(Strict):
    tpm: int | None = None
    rpm: int | None = None


class Step(Strict):
    kind: Literal["check", "success", "redis"]
    model_id: str | None = None
    model: str | None = None
    total_tokens: int = 0
    redis: RedisMode = "none"


class ErrorFields(Strict):
    status_code: int
    code: str
    error_type: str
    retry_after_seconds: int
    message: str
    content: str
    num_retries: int | None


class StepOutput(Strict):
    kind: Literal["check", "success", "redis"]
    allowed: bool | None = None
    error: ErrorFields | None = None


class Output(Strict):
    steps: list[StepOutput]


class Check(Protocol):
    async def async_pre_call_check(self, deployment: dict[str, object], parent_otel_span: object = None) -> object: ...

    async def async_log_success_event(
        self,
        kwargs: dict[str, object],
        response_obj: object,
        start_time: object,
        end_time: object,
    ) -> None: ...


@final
class FakeRedisStore:
    def __init__(self, values: dict[str, int], mode: Literal["up", "breaker", "down"]) -> None:
        self.values: dict[str, int] = values
        self.mode: Literal["up", "breaker", "down"] = mode

    def _read(self) -> None:
        # RedisCache's get methods swallow a connection failure and answer None; only the breaker guard raises
        if self.mode == "breaker":
            raise RedisCircuitBreakerOpenError

    def _write(self) -> None:
        if self.mode == "breaker":
            raise RedisCircuitBreakerOpenError
        if self.mode == "down":
            raise RuntimeError("redis is down")

    def get_cache(self, key: str, parent_otel_span: object = None) -> int | None:
        _ = parent_otel_span
        self._read()
        if self.mode == "down":
            return None
        return self.values.get(key)

    async def async_get_cache(self, key: str, parent_otel_span: object = None) -> int | None:
        _ = parent_otel_span
        self._read()
        if self.mode == "down":
            return None
        return self.values.get(key)

    def increment_cache(self, key: str, value: float, **_kwargs: object) -> int:
        self._write()
        current = self.values.get(key, 0) + int(value)
        self.values[key] = current
        return current

    async def async_increment(
        self,
        key: str,
        value: float,
        parent_otel_span: object = None,
        **_kwargs: object,
    ) -> int:
        _ = parent_otel_span
        self._write()
        current = self.values.get(key, 0) + int(value)
        self.values[key] = current
        return current


def build_redis(store: FakeRedisStore) -> RedisCache:
    cache = object.__new__(RedisCache)
    object.__setattr__(cache, "get_cache", store.get_cache)
    object.__setattr__(cache, "async_get_cache", store.async_get_cache)
    object.__setattr__(cache, "increment_cache", store.increment_cache)
    object.__setattr__(cache, "async_increment", store.async_increment)
    return cache


def deployment_dict(deployment: DeploymentInput) -> dict[str, object]:
    model_info: dict[str, object] = {}
    params: dict[str, object] = {}
    if deployment.id is not None:
        model_info["id"] = deployment.id
    if deployment.model is not None:
        params["model"] = deployment.model
    payload: dict[str, object] = {
        "model_name": deployment.model_name,
        "model_info": model_info,
        "litellm_params": params,
    }
    payload.update(
        {
            field: value
            for field, value in (
                ("tpm", deployment.tpm),
                ("rpm", deployment.rpm),
                ("itpm", deployment.itpm),
            )
            if value is not None
        }
    )
    return payload


def cache_keys(deployment: DeploymentInput, minute: str) -> tuple[str, str]:
    return (
        f"{deployment.id}:{deployment.model}:tpm:{minute}",
        f"{deployment.id}:{deployment.model}:rpm:{minute}",
    )


def error_fields(error: RateLimitError) -> ErrorFields:
    return ErrorFields(
        status_code=error.status_code,
        code="" if error.code is None else error.code,
        error_type="" if error.type is None else error.type,
        retry_after_seconds=int(error.response.headers["retry-after"]),
        message=error.message,
        content=error.response.content.decode(),
        num_retries=error.num_retries,
    )


@final
class Scenario:
    def __init__(self, spec: "ModelRateLimits") -> None:
        self.spec = spec
        self.now = datetime.fromtimestamp(spec.now_millis / MILLIS_PER_SECOND, UTC)
        self.deployment = deployment_dict(spec.deployment)
        keys = cache_keys(spec.deployment, self.now.strftime("%H-%M"))
        memory: Final = InMemoryCache()
        if spec.local.tpm is not None:
            memory.set_cache(key=keys[0], value=spec.local.tpm, ttl=ROUTING_ARGS_TTL)
        if spec.local.rpm is not None:
            memory.set_cache(key=keys[1], value=spec.local.rpm, ttl=ROUTING_ARGS_TTL)
        self.store: FakeRedisStore | None = None
        if spec.redis == "none":
            self.cache = DualCache(in_memory_cache=memory)
        else:
            values: dict[str, int] = {}
            if spec.remote.tpm is not None:
                values[keys[0]] = spec.remote.tpm
            if spec.remote.rpm is not None:
                values[keys[1]] = spec.remote.rpm
            store = FakeRedisStore(values, spec.redis)
            self.store = store
            self.cache = DualCache(in_memory_cache=memory, redis_cache=build_redis(store))
        self.check: Final[Check] = ModelRateLimitingCheck(self.cache)

    async def run(self) -> Output:
        with (
            patch.object(model_check, "get_utc_datetime", return_value=self.now),
            patch.object(io_check, "get_utc_datetime", return_value=self.now),
        ):
            steps = [await self.run_step(step) for step in self.spec.steps]
        return Output(steps=steps)

    async def run_step(self, step: Step) -> StepOutput:
        if step.kind == "redis":
            store = self.store
            if store is not None and step.redis != "none":
                store.mode = step.redis
            return StepOutput(kind=step.kind)
        if step.kind == "success":
            event: dict[str, object] = {
                "model_id": step.model_id,
                "total_tokens": step.total_tokens,
                "hidden_params": {"litellm_model_name": step.model},
            }
            kwargs: dict[str, object] = {"standard_logging_object": event}
            await self.check.async_log_success_event(kwargs, None, None, None)
            return StepOutput(kind=step.kind)
        try:
            _ = await self.check.async_pre_call_check(self.deployment, None)
        except RateLimitError as raised:
            return StepOutput(kind=step.kind, allowed=False, error=error_fields(raised))
        return StepOutput(kind=step.kind, allowed=True)


class ModelRateLimits(Operation):
    now_millis: int
    deployment: DeploymentInput
    local: Counters = Field(default_factory=Counters)
    remote: Counters = Field(default_factory=Counters)
    redis: RedisMode = "none"
    steps: list[Step]

    @override
    def run(self) -> Output:
        return asyncio.run(Scenario(self).run())
