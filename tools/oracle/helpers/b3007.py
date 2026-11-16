import asyncio
import os
from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.types.agents import AgentResponse
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

TTL_VARIABLE: str = "LITELLM_MAX_BUDGET_PER_SESSION_TTL"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
FLOAT: TypeAdapter[float] = TypeAdapter(float)
INT: TypeAdapter[int] = TypeAdapter(int)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)

RedisMode: TypeAlias = Literal["absent", "healthy", "read_fails", "script_fails"]


class PreCall(TypedDict):
    agent_id: NotRequired[str]
    data: dict[str, JsonValue]


class Charge(TypedDict):
    litellm_params: NotRequired[dict[str, JsonValue]]
    response_cost: NotRequired[float | None]


class Input(TypedDict):
    ttl_env: NotRequired[str]
    redis: NotRequired[RedisMode]
    memory: NotRequired[dict[str, float]]
    redis_values: NotRequired[dict[str, str]]
    agents: NotRequired[list[dict[str, JsonValue]]]
    pre_calls: NotRequired[list[PreCall]]
    charges: NotRequired[list[Charge]]


class Output(TypedDict):
    ttl: int
    script_registered: bool
    calls: list[JsonValue]
    memory: dict[str, float]
    memory_ttls: dict[str, int]
    redis: dict[str, float]
    redis_ttls: dict[str, int]


@runtime_checkable
class Script(Protocol):
    async def __call__(self, keys: list[str], args: list[str | int]) -> str: ...


@runtime_checkable
class RedisLike(Protocol):
    async def async_get_cache(self, key: str) -> str | None: ...

    def async_register_script(self, _script: str) -> Script: ...


@runtime_checkable
class DualCacheLike(Protocol):
    redis_cache: RedisLike | None


@runtime_checkable
class UsageCacheLike(Protocol):
    dual_cache: DualCacheLike

    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> float | None: ...

    async def async_set_cache(
        self,
        key: str,
        *,
        value: float,
        ttl: int,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None: ...


@runtime_checkable
class UserKeyLike(Protocol):
    agent_id: str | None


@runtime_checkable
class Handler(Protocol):
    ttl: int
    increment_script: Script | None

    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserKeyLike,
        cache: DualCacheLike,
        data: dict[str, JsonValue],
        call_type: str,
    ) -> None: ...

    async def async_log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: JsonValue,
        start_time: JsonValue,
        end_time: JsonValue,
    ) -> None: ...


class RateLimitFields(BaseModel):
    status_code: int
    detail: JsonValue
    category: str
    rate_limit_type: str | None
    model: str
    llm_provider: str


class UserKey(UserKeyLike):
    def __init__(self, agent_id: str | None) -> None:
        self.agent_id = agent_id


class FakeScript(Script):
    def __init__(self, store: dict[str, str], expires: dict[str, int], *, fail: bool) -> None:
        self.store = store
        self.expires = expires
        self.fail = fail

    @override
    async def __call__(self, keys: list[str], args: list[str | int]) -> str:
        if self.fail:
            raise RuntimeError("the fake Redis script failed")
        key = keys[0]
        existed = key in self.store
        current = FLOAT.validate_python(self.store[key]) if existed else 0.0
        total = current + FLOAT.validate_python(args[0])
        self.store[key] = str(total)
        if not existed:
            self.expires[key] = INT.validate_python(args[1])
        return str(total)


class FakeRedis(RedisLike):
    def __init__(
        self,
        *,
        read_fails: bool,
        script_fails: bool,
        values: dict[str, str],
    ) -> None:
        self.read_fails = read_fails
        self.script_fails = script_fails
        self.store = values.copy()
        self.expires: dict[str, int] = {}

    @override
    async def async_get_cache(self, key: str) -> str | None:
        if self.read_fails:
            raise RuntimeError("the fake Redis read failed")
        return self.store.get(key)

    @override
    def async_register_script(self, _script: str) -> Script:
        fail = self.script_fails
        return FakeScript(self.store, self.expires, fail=fail)


class FakeDualCache(DualCacheLike):
    def __init__(self, redis_cache: RedisLike | None) -> None:
        self.redis_cache = redis_cache


class FakeUsageCache(UsageCacheLike):
    def __init__(self, dual_cache: FakeDualCache, memory: dict[str, float]) -> None:
        self.dual_cache: DualCacheLike = dual_cache
        self.memory = memory.copy()
        self.memory_ttls: dict[str, int] = {}

    @override
    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> float | None:
        if litellm_parent_otel_span is not None or not local_only:
            raise RuntimeError("the session budget hook reads the local cache only")
        return self.memory.get(key)

    @override
    async def async_set_cache(
        self,
        key: str,
        *,
        value: float,
        ttl: int,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        if litellm_parent_otel_span is not None or not local_only:
            raise RuntimeError("the session budget hook writes the local cache only")
        self.memory[key] = value
        self.memory_ttls[key] = ttl


BUILD_HANDLER: Callable[[UsageCacheLike], Handler] = TypeAdapter(
    Callable[[UsageCacheLike], Handler]
).validate_python(PROXY_HOOKS["max_budget_per_session_limiter"])


def seed_registry(agents: list[dict[str, JsonValue]]) -> None:
    global_agent_registry.reset_agent_list()
    for agent in agents:
        global_agent_registry.register_agent(AGENT.validate_python(agent))


def exceeded_outcome(fields: RateLimitFields) -> JsonValue:
    return JSON.validate_python(
        {
            "kind": "pre_call",
            "outcome": "exceeded",
            "status_code": fields.status_code,
            "detail": fields.detail,
            "category": fields.category,
            "rate_limit_type": fields.rate_limit_type,
            "model": fields.model,
            "llm_provider": fields.llm_provider,
        }
    )


async def pre_call_outcome(
    handler: Handler,
    dual_cache: DualCacheLike,
    call: PreCall,
) -> JsonValue:
    agent_id = call.get("agent_id")
    try:
        await handler.async_pre_call_hook(
            user_api_key_dict=UserKey(agent_id),
            cache=dual_cache,
            data=call["data"],
            call_type="completion",
        )
    except ProxyRateLimitError as error:
        return exceeded_outcome(RateLimitFields.model_validate(error, from_attributes=True))
    return JSON.validate_python({"kind": "pre_call", "outcome": "admitted"})


async def charge_outcome(handler: Handler, call: Charge) -> JsonValue:
    kwargs: dict[str, JsonValue] = {}
    litellm_params = call.get("litellm_params")
    if litellm_params is not None:
        kwargs["litellm_params"] = litellm_params
    response_cost = call.get("response_cost")
    if response_cost is not None:
        kwargs["response_cost"] = response_cost
    await handler.async_log_success_event(
        kwargs=kwargs,
        response_obj=None,
        start_time=None,
        end_time=None,
    )
    return JSON.validate_python({"kind": "charge", "outcome": "logged"})


def redis_amounts(redis: FakeRedis | None) -> dict[str, float]:
    if redis is None:
        return {}
    return {key: FLOAT.validate_python(text) for key, text in redis.store.items()}


def redis_ttls(redis: FakeRedis | None) -> dict[str, int]:
    if redis is None:
        return {}
    return redis.expires


async def execute(parsed: Input) -> Output:
    ttl_text = parsed.get("ttl_env")
    if ttl_text is None:
        os.environ.pop(TTL_VARIABLE, None)
    else:
        os.environ[TTL_VARIABLE] = ttl_text
    agents = parsed.get("agents")
    if agents is None:
        agents = []
    seed_registry(agents)
    mode = parsed.get("redis")
    if mode is None:
        mode = "absent"
    redis_values = parsed.get("redis_values")
    if redis_values is None:
        redis_values = {}
    redis: FakeRedis | None = None
    if mode != "absent":
        redis = FakeRedis(
            read_fails=mode == "read_fails",
            script_fails=mode == "script_fails",
            values=redis_values,
        )
    memory = parsed.get("memory")
    if memory is None:
        memory = {}
    cache = FakeUsageCache(FakeDualCache(redis), memory)
    handler = BUILD_HANDLER(cache)
    outcomes: list[JsonValue] = []
    pre_calls = parsed.get("pre_calls")
    if pre_calls is None:
        pre_calls = []
    for pre_call in pre_calls:
        outcomes.append(await pre_call_outcome(handler, cache.dual_cache, pre_call))
    charges = parsed.get("charges")
    if charges is None:
        charges = []
    for charge in charges:
        outcomes.append(await charge_outcome(handler, charge))
    return {
        "ttl": handler.ttl,
        "script_registered": handler.increment_script is not None,
        "calls": outcomes,
        "memory": cache.memory,
        "memory_ttls": cache.memory_ttls,
        "redis": redis_amounts(redis),
        "redis_ttls": redis_ttls(redis),
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = TypeAdapter(Input).validate_python(input_value)
    return JSON.validate_python(asyncio.run(execute(parsed)))
