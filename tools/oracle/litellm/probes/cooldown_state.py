import asyncio
import time
from typing import Final, Literal, Protocol, final
from unittest.mock import patch

from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.router import Router
from litellm.router_utils.cooldown_cache import CooldownCache, CooldownCacheValue
from litellm.router_utils.health_state_cache import (
    DeploymentHealthCache,
    DeploymentHealthStateValue,
)
from litellm.types.router import AllowedFailsPolicy, RouterRateLimitError
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

from probes.protocol import Operation, Strict

# pyright: strict, reportMissingTypeStubs=false

API_KEY = "sk-probe"
COOLDOWN_PREFIX: Final = "deployment:"
COOLDOWN_SUFFIX: Final = ":cooldown"


class SettingsInput(Strict):
    default_cooldown_time: float
    redis_read_interval_seconds: float
    staleness_threshold: float


class CooldownInput(Strict):
    exception_received: str
    status_code: str
    timestamp: float
    cooldown_time: float


class HealthInput(Strict):
    is_healthy: bool = True
    timestamp: float = 0.0
    reason: str = ""


class DeploymentInput(Strict):
    model_name: str
    id: str


class HealthSettings(Strict):
    enabled: bool = False
    has_allowed_fails_policy: bool = False
    scoped_groups: list[str] | None = None


class Step(Strict):
    kind: Literal["add", "read", "minimum", "set_health", "health", "filter"]
    now_millis: int
    model_id: str = ""
    model_ids: list[str] = Field(default_factory=list)
    status_code: int = 0
    cooldown_time: float | None = None
    exception: str = ""
    states: dict[str, HealthInput] = Field(default_factory=dict)
    health_check_probe: bool = False


class ActiveView(Strict):
    model_id: str
    remaining_seconds: str
    exception_received: str
    status_code: str
    timestamp: str
    cooldown_time: str


# fixed-point text: the reply buffer turns a JSON float into a map
class StepOutput(Strict):
    kind: Literal["add", "read", "minimum", "set_health", "health", "filter"]
    local_expiry: str | None = None
    local_expiries: list[str | None] | None = None
    active: list[ActiveView] | None = None
    minimum: str | None = None
    unhealthy: list[str] | None = None
    kept: list[str] | None = None


class Output(Strict):
    steps: list[StepOutput]


class KeptInfo(TypedDict):
    id: str


class KeptDeployment(TypedDict):
    model_info: KeptInfo


KEPT: Final = TypeAdapter(list[KeptDeployment])


class RouterView(Protocol):
    async def async_get_healthy_deployments(
        self,
        model: str,
        request_kwargs: dict[str, object],
        *,
        health_check_probe: bool = False,
    ) -> list[dict[str, JsonValue]] | dict[str, JsonValue]: ...


@final
class FakeRedisStore:
    def __init__(self, values: dict[str, object]) -> None:
        self.values = values

    def get_cache(self, key: str, parent_otel_span: object = None) -> object:
        _ = parent_otel_span
        return self.values.get(key)

    async def async_get_cache(self, key: str, parent_otel_span: object = None) -> object:
        _ = parent_otel_span
        return self.values.get(key)

    def batch_get_cache(self, key_list: list[str], parent_otel_span: object = None) -> dict[str, object]:
        _ = parent_otel_span
        return {key: self.values[key] for key in key_list if key in self.values}

    async def async_batch_get_cache(self, key_list: list[str], parent_otel_span: object = None) -> dict[str, object]:
        _ = parent_otel_span
        return {key: self.values[key] for key in key_list if key in self.values}

    def set_cache(self, key: str, value: object, **_kwargs: object) -> None:
        self.values[key] = value


def build_redis(store: FakeRedisStore) -> RedisCache:
    cache = object.__new__(RedisCache)
    object.__setattr__(cache, "get_cache", store.get_cache)
    object.__setattr__(cache, "async_get_cache", store.async_get_cache)
    object.__setattr__(cache, "batch_get_cache", store.batch_get_cache)
    object.__setattr__(cache, "async_batch_get_cache", store.async_batch_get_cache)
    object.__setattr__(cache, "set_cache", store.set_cache)
    return cache


def cooldown_key(model_id: str) -> str:
    return f"{COOLDOWN_PREFIX}{model_id}{COOLDOWN_SUFFIX}"


def cooldown_entry(record: CooldownInput) -> CooldownCacheValue:
    return CooldownCacheValue(
        exception_received=record.exception_received,
        status_code=record.status_code,
        timestamp=record.timestamp,
        cooldown_time=record.cooldown_time,
    )


def health_entry(state: HealthInput) -> DeploymentHealthStateValue:
    return DeploymentHealthStateValue(is_healthy=state.is_healthy, timestamp=state.timestamp, reason=state.reason)


def model_entry(deployment: DeploymentInput) -> dict[str, object]:
    return {
        "model_name": deployment.model_name,
        "litellm_params": {"model": f"openai/{deployment.id}", "api_key": API_KEY},
        "model_info": {"id": deployment.id},
    }


def kept_ids(deployments: list[dict[str, JsonValue]] | dict[str, JsonValue]) -> list[str]:
    entries: list[dict[str, JsonValue]] = [deployments] if isinstance(deployments, dict) else deployments
    return [entry["model_info"]["id"] for entry in KEPT.validate_python(entries)]


async def local_expiry(store: DualCache, key: str) -> str | None:
    expiry = await store.in_memory_cache.async_get_ttl(key)
    return None if expiry is None else f"{float(expiry):.6f}"


def active_view(model_id: str, value: CooldownCacheValue) -> ActiveView:
    remaining = (value["timestamp"] + value["cooldown_time"]) - time.time()
    return ActiveView(
        model_id=model_id,
        remaining_seconds=f"{remaining:.6f}",
        exception_received=value["exception_received"],
        status_code=value["status_code"],
        timestamp=f"{value['timestamp']:.6f}",
        cooldown_time=f"{value['cooldown_time']:.6f}",
    )


@final
class Scenario:
    def __init__(self, spec: "CooldownState") -> None:
        self.spec = spec
        values: dict[str, object] = {}
        for model_id, record in spec.shared_cooldowns.items():
            values[cooldown_key(model_id)] = cooldown_entry(record)
        if spec.shared_health:
            snapshot: dict[str, object] = {}
            for model_id, state in spec.shared_health.items():
                snapshot[model_id] = health_entry(state)
            values[DeploymentHealthCache.CACHE_KEY] = snapshot
        self.store = FakeRedisStore(values)
        redis = build_redis(self.store)
        self.cooldowns = CooldownCache(
            cache=DualCache(in_memory_cache=InMemoryCache(), redis_cache=redis),
            default_cooldown_time=spec.settings.default_cooldown_time,
            redis_read_interval_seconds=spec.settings.redis_read_interval_seconds,
        )
        self.router = Router(
            model_list=[model_entry(deployment) for deployment in spec.deployments],
            enable_health_check_routing=spec.health.enabled,
            allowed_fails_policy=AllowedFailsPolicy() if spec.health.has_allowed_fails_policy else None,
            background_health_check_model_groups=spec.health.scoped_groups,
        )
        self.router.health_state_cache = DeploymentHealthCache(
            cache=DualCache(in_memory_cache=InMemoryCache(), redis_cache=redis),
            staleness_threshold=spec.settings.staleness_threshold,
        )

    async def run(self) -> Output:
        steps: list[StepOutput] = []
        for step in self.spec.steps:
            with patch("time.time", return_value=step.now_millis / 1000):
                steps.append(await self.run_step(step))
        return Output(steps=steps)

    async def run_step(self, step: Step) -> StepOutput:
        if step.kind == "add":
            self.cooldowns.add_deployment_to_cooldown(
                model_id=step.model_id,
                original_exception=Exception(step.exception),
                exception_status=step.status_code,
                cooldown_time=step.cooldown_time,
            )
            return StepOutput(
                kind=step.kind,
                local_expiry=await local_expiry(self.cooldowns.cooldown_store, cooldown_key(step.model_id)),
            )
        if step.kind == "read":
            active = self.cooldowns.get_active_cooldowns(step.model_ids, None)
            return StepOutput(
                kind=step.kind,
                active=[active_view(model_id, value) for model_id, value in active],
                local_expiries=[
                    await local_expiry(self.cooldowns.cooldown_store, cooldown_key(model_id))
                    for model_id in step.model_ids
                ],
            )
        if step.kind == "minimum":
            minimum = self.cooldowns.get_min_cooldown(step.model_ids, None)
            return StepOutput(kind=step.kind, minimum=f"{minimum:.6f}")
        if step.kind == "set_health":
            states: dict[str, DeploymentHealthStateValue] = {}
            for model_id, state in step.states.items():
                states[model_id] = health_entry(state)
            self.router.health_state_cache.set_deployment_health_states(states)
            return StepOutput(
                kind=step.kind,
                local_expiry=await local_expiry(
                    self.router.health_state_cache.cache, DeploymentHealthCache.CACHE_KEY
                ),
            )
        if step.kind == "health":
            return StepOutput(
                kind=step.kind,
                unhealthy=sorted(self.router.health_state_cache.get_unhealthy_deployment_ids()),
            )
        return StepOutput(kind=step.kind, kept=await self.kept(self.router, step))

    async def kept(self, router: RouterView, step: Step) -> list[str]:
        surviving: set[str] = set()
        for deployment in self.spec.deployments:
            try:
                deployments = await router.async_get_healthy_deployments(
                    deployment.model_name, {}, health_check_probe=step.health_check_probe
                )
            except RouterRateLimitError:  # the public entry reports an empty pool as this error
                continue
            surviving.update(kept_ids(deployments))
        return [deployment.id for deployment in self.spec.deployments if deployment.id in surviving]


class CooldownState(Operation):
    settings: SettingsInput
    shared_cooldowns: dict[str, CooldownInput] = Field(default_factory=dict)
    shared_health: dict[str, HealthInput] = Field(default_factory=dict)
    deployments: list[DeploymentInput] = Field(default_factory=list)
    health: HealthSettings = Field(default_factory=HealthSettings)
    steps: list[Step]

    @override
    def run(self) -> Output:
        return asyncio.run(Scenario(self).run())
