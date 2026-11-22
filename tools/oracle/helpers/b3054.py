import asyncio
import os
from collections.abc import Callable
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.types.agents import AgentResponse
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

TTL_VARIABLE: str = "LITELLM_MAX_ITERATIONS_TTL"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
AGENT: TypeAdapter[AgentResponse] = TypeAdapter(AgentResponse)

RedisMode: TypeAlias = Literal["absent", "healthy", "script_fails"]


class Call(TypedDict):
    data: dict[str, JsonValue]
    agent_id: NotRequired[str]
    key_metadata: NotRequired[dict[str, JsonValue]]


class Input(TypedDict):
    calls: NotRequired[list[Call]]
    agents: NotRequired[list[dict[str, JsonValue]]]
    redis: NotRequired[RedisMode]
    ttl_env: NotRequired[str]
    memory: NotRequired[dict[str, int]]
    redis_values: NotRequired[dict[str, int]]


class Output(TypedDict):
    ttl: int
    script_registered: bool
    calls: list[JsonValue]
    memory: dict[str, int]
    memory_ttls: dict[str, int]
    redis: dict[str, int]
    redis_ttls: dict[str, int]


class Script(Protocol):
    async def __call__(self, keys: list[str], args: list[int]) -> str: ...


class RedisCache(Protocol):
    def async_register_script(self, _script: str) -> Script: ...


class UserKey:
    def __init__(self, agent_id: str | None, metadata: dict[str, JsonValue]) -> None:
        self.agent_id = agent_id
        self.metadata = metadata


class FakeRedis(RedisCache):
    def __init__(self, *, mode: RedisMode, values: dict[str, int]) -> None:
        self.mode = mode
        self.store = values.copy()
        self.expires: dict[str, int] = {}

    @override
    def async_register_script(self, _script: str) -> Script:
        return self.run_script

    async def run_script(self, keys: list[str], args: list[int]) -> str:
        if self.mode == "script_fails":
            raise RuntimeError("the fake Redis script failed")
        key = keys[0]
        current = self.store.get(key)
        total = 1 if current is None else current + 1
        self.store[key] = total
        if total == 1:
            self.expires[key] = args[0]
        return str(total)


def redis_state(server: FakeRedis | None) -> dict[str, int]:
    if server is None:
        return {}
    return server.store


def redis_expiry(server: FakeRedis | None) -> dict[str, int]:
    if server is None:
        return {}
    return server.expires


class FakeUsageCache:
    def __init__(self, redis: RedisCache | None, memory: dict[str, int]) -> None:
        self.dual_cache = self
        self.redis_cache: RedisCache | None = redis
        self.memory = memory.copy()
        self.memory_ttls: dict[str, int] = {}

    async def async_get_cache(
        self, key: str, *, litellm_parent_otel_span: JsonValue, local_only: bool
    ) -> int | None:
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the max iterations hook reads without a span")
        if not local_only:
            raise RuntimeError("the max iterations hook reads the local cache only")
        return self.memory.get(key)

    async def async_set_cache(
        self,
        key: str,
        *,
        value: int,
        ttl: int,
        litellm_parent_otel_span: JsonValue,
        local_only: bool,
    ) -> None:
        if litellm_parent_otel_span is not None:
            raise RuntimeError("the max iterations hook writes without a span")
        if not local_only:
            raise RuntimeError("the max iterations hook writes the local cache only")
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


class RateLimitFields(BaseModel):
    status_code: int
    category: str
    rate_limit_type: str | None
    model: str
    llm_provider: str
    detail: str


BUILD_HANDLER: Callable[[FakeUsageCache], Handler] = TypeAdapter(
    Callable[[FakeUsageCache], Handler]
).validate_python(PROXY_HOOKS["max_iterations_limiter"])


def seed_environment(parsed: Input) -> None:
    ttl_text = parsed.get("ttl_env")
    if ttl_text is None:
        os.environ.pop(TTL_VARIABLE, None)
    else:
        os.environ[TTL_VARIABLE] = ttl_text
    agents = parsed.get("agents")
    if agents is None:
        agents = []
    for agent in agents:
        global_agent_registry.register_agent(AGENT.validate_python(agent))


def build_cache(parsed: Input) -> tuple[FakeUsageCache, FakeRedis | None]:
    memory = parsed.get("memory")
    if memory is None:
        memory = {}
    mode = parsed.get("redis")
    if mode is None or mode == "absent":
        return FakeUsageCache(None, memory), None
    values = parsed.get("redis_values")
    if values is None:
        values = {}
    server = FakeRedis(mode=mode, values=values)
    return FakeUsageCache(server, memory), server


async def pre_call_outcome(
    handler: Handler,
    cache: FakeUsageCache,
    call: Call,
) -> JsonValue:
    metadata = call.get("key_metadata")
    key = UserKey(call.get("agent_id"), metadata if metadata is not None else {})
    try:
        await handler.async_pre_call_hook(
            user_api_key_dict=key,
            cache=cache,
            data=call["data"],
            call_type="completion",
        )
    except HTTPException as error:
        fields = RateLimitFields.model_validate(error, from_attributes=True)
        exceeded: dict[str, JsonValue] = {
            "kind": "pre_call",
            "outcome": "exceeded",
            "status_code": fields.status_code,
            "category": fields.category,
            "rate_limit_type": fields.rate_limit_type,
            "model": fields.model,
            "llm_provider": fields.llm_provider,
            "detail": fields.detail,
        }
        return JSON.validate_python(exceeded)
    admitted: dict[str, JsonValue] = {"kind": "pre_call", "outcome": "admitted"}
    return JSON.validate_python(admitted)


async def execute(parsed: Input) -> Output:
    seed_environment(parsed)
    cache, server = build_cache(parsed)
    handler = BUILD_HANDLER(cache)
    calls = parsed.get("calls")
    if calls is None:
        calls = []
    outcomes = [await pre_call_outcome(handler, cache, call) for call in calls]
    return {
        "ttl": handler.ttl,
        "script_registered": handler.increment_script is not None,
        "calls": outcomes,
        "memory": cache.memory,
        "memory_ttls": cache.memory_ttls,
        "redis": redis_state(server),
        "redis_ttls": redis_expiry(server),
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = TypeAdapter(Input).validate_python(input_value)
    return JSON.validate_python(asyncio.run(execute(parsed)))
