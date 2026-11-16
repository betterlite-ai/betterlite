import asyncio
import os
from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

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


class Script(Protocol):
    async def __call__(self, keys: list[str], args: list[str | int]) -> str: ...


class RedisLike(Protocol):
    async def async_get_cache(self, key: str) -> str | None: ...

    def async_register_script(self, _script: str) -> Script: ...


class UserKey:
    def __init__(self, agent_id: str | None) -> None:
        self.agent_id = agent_id


class FakeScript:
    def __init__(self, store: dict[str, str], expires: dict[str, int], *, fail: bool) -> None:
        self.store = store
        self.expires = expires
        self.fail = fail

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
    def __init__(self, *, mode: RedisMode, values: dict[str, str]) -> None:
        self.mode = mode
        self.store = values.copy()
        self.expires: dict[str, int] = {}

    @override
    async def async_get_cache(self, key: str) -> str | None:
        if self.mode == "read_fails":
            raise RuntimeError("the fake Redis read failed")
        return self.store.get(key)

    @override
    def async_register_script(self, _script: str) -> Script:
        return FakeScript(self.store, self.expires, fail=self.mode == "script_fails")


class FakeUsageCache:
    def __init__(self, redis: RedisLike | None, memory: dict[str, float]) -> None:
        self.dual_cache = self
        self.redis_cache: RedisLike | None = redis
        self.memory = memory.copy()
        self.memory_ttls: dict[str, int] = {}

    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> float | None:
        if litellm_parent_otel_span is not None or not local_only:
            raise RuntimeError("the session budget hook reads the local cache only")
        return self.memory.get(key)

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


class Handler(Protocol):
    ttl: int
    increment_script: Script | None

    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserKey,
        cache: FakeUsageCache,
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
    category: str
    rate_limit_type: str | None
    model: str
    llm_provider: str


BUILD_HANDLER: Callable[[FakeUsageCache], Handler] = TypeAdapter(
    Callable[[FakeUsageCache], Handler]
).validate_python(PROXY_HOOKS["max_budget_per_session_limiter"])


def seed_registry(agents: list[dict[str, JsonValue]]) -> None:
    global_agent_registry.reset_agent_list()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/agent_endpoints/agent_registry.py#L430
    for agent in agents:
        global_agent_registry.register_agent(AGENT.validate_python(agent))


def exceeded_outcome(fields: RateLimitFields) -> JsonValue:
    outcome: dict[str, JsonValue] = {
        "kind": "pre_call",
        "outcome": "exceeded",
        "status_code": fields.status_code,
        "category": fields.category,
        "rate_limit_type": fields.rate_limit_type,
        "model": fields.model,
        "llm_provider": fields.llm_provider,
    }
    return JSON.validate_python(outcome)


def plain_outcome(kind: str, outcome: str) -> JsonValue:
    value: dict[str, JsonValue] = {"kind": kind, "outcome": outcome}
    return JSON.validate_python(value)


async def pre_call_outcome(
    handler: Handler,
    cache: FakeUsageCache,
    call: PreCall,
) -> JsonValue:
    agent_id = call.get("agent_id")
    try:
        await handler.async_pre_call_hook(
            user_api_key_dict=UserKey(agent_id),
            cache=cache,
            data=call["data"],
            call_type="completion",
        )
    except ProxyRateLimitError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L101
        return exceeded_outcome(RateLimitFields.model_validate(error, from_attributes=True))
    return plain_outcome("pre_call", "admitted")


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
    return plain_outcome("charge", "logged")


def seed_environment(parsed: Input) -> None:
    ttl_text = parsed.get("ttl_env")
    if ttl_text is None:
        os.environ.pop(TTL_VARIABLE, None)
    else:
        os.environ[TTL_VARIABLE] = ttl_text
    agents = parsed.get("agents")
    if agents is None:
        agents = []
    seed_registry(agents)


def build_cache(parsed: Input) -> tuple[FakeUsageCache, FakeRedis | None]:
    mode = parsed.get("redis")
    if mode is None:
        mode = "absent"
    memory = parsed.get("memory")
    if memory is None:
        memory = {}
    redis: FakeRedis | None = None
    if mode != "absent":
        redis_values = parsed.get("redis_values")
        if redis_values is None:
            redis_values = {}
        redis = FakeRedis(mode=mode, values=redis_values)
    return FakeUsageCache(redis, memory), redis


async def execute(parsed: Input) -> Output:
    seed_environment(parsed)
    cache, redis = build_cache(parsed)
    handler = BUILD_HANDLER(cache)
    pre_calls = parsed.get("pre_calls")
    if pre_calls is None:
        pre_calls = []
    outcomes = [await pre_call_outcome(handler, cache, call) for call in pre_calls]
    charges = parsed.get("charges")
    if charges is None:
        charges = []
    outcomes.extend([await charge_outcome(handler, call) for call in charges])
    redis_store: dict[str, str] = {} if redis is None else redis.store
    redis_expires: dict[str, int] = {} if redis is None else redis.expires
    return {
        "ttl": handler.ttl,
        "script_registered": handler.increment_script is not None,
        "calls": outcomes,
        "memory": cache.memory,
        "memory_ttls": cache.memory_ttls,
        "redis": {key: FLOAT.validate_python(text) for key, text in redis_store.items()},
        "redis_ttls": redis_expires,
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = TypeAdapter(Input).validate_python(input_value)
    return JSON.validate_python(asyncio.run(execute(parsed)))
