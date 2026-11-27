import asyncio
import logging
from collections.abc import Mapping, Sequence
from typing import Final, NotRequired, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.constants import LOGIN_THROTTLE_MAX_TRACKED_BLOCKS as MAX_BLOCKS
from litellm.constants import LOGIN_THROTTLE_MAX_TRACKED_COUNTERS as MAX_COUNTERS
from litellm.proxy.auth.login_throttle import DEFAULT_FAILED_LOGIN_BLOCK_SECONDS as DEFAULT_BLOCK
from litellm.proxy.auth.login_throttle import DEFAULT_FAILED_LOGIN_WINDOW_SECONDS as DEFAULT_WINDOW
from pydantic import TypeAdapter
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import override

MALFORMED: Final = "malformed"
EXPIRED: Final = -2
NO_EXPIRY: Final = -1.0
TTL_KEYS: Final = TypeAdapter(tuple[str, str, str, str])
SCRIPT_ARGS: Final = TypeAdapter(tuple[int, ...])


class RedisInput(TypedDict):
    faults: NotRequired[dict[str, str]]


class Clock:
    def __init__(self, now: float) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


class CaptureHandler(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def drain(self) -> list[str]:
        messages = self.messages
        self.messages = []
        return messages


class RedisState:
    def __init__(self, clock: Clock, faults: Mapping[str, str]) -> None:
        self.clock = clock
        self.faults = faults
        self.counters: dict[str, tuple[int, float]] = {}
        self.blocks: dict[str, float] = {}

    def block_ttl(self, key: str) -> int:
        now = self.clock.now
        expiry = self.blocks.get(key)
        if expiry is None or expiry <= now:
            return EXPIRED
        return max(int(expiry - now), 0)

    def bump(self, keys: tuple[str, str], limits: tuple[int, int, int]) -> int:
        counter_key, block_key = keys
        limit, window, block = limits
        now = self.clock.now
        blocked = self.block_ttl(block_key)
        if blocked > 0:
            return blocked
        entry = self.counters.get(counter_key)
        count, expiry = (0, NO_EXPIRY) if entry is None else entry
        if expiry <= now:
            count = 0
            expiry = NO_EXPIRY
        count += 1
        if expiry < 0:
            expiry = now + window
        self.counters[counter_key] = (count, expiry)
        if count > limit:
            self.blocks[block_key] = now + block
            return block
        return 0


class Script:
    def __init__(self, state: RedisState) -> None:
        self.state = state

    async def __call__(self, keys: Sequence[str], args: Sequence[int]) -> list[int]:
        await asyncio.sleep(0)
        key_values = TTL_KEYS.validate_python(keys)
        arg_values = SCRIPT_ARGS.validate_python(args)
        if not arg_values:
            return self.block_ttls(key_values)
        return self.record(key_values, arg_values)

    def block_ttls(self, keys: tuple[str, str, str, str]) -> list[int]:
        fault = self.state.faults.get("block_ttls")
        if fault == MALFORMED:
            return [1]
        if fault is not None:
            raise RedisConnectionError("injected fault")
        return [self.state.block_ttl(keys[1]), self.state.block_ttl(keys[3])]

    def record(self, keys: tuple[str, str, str, str], args: tuple[int, ...]) -> list[int]:
        fault = self.state.faults.get("record_failure")
        if fault == MALFORMED:
            return [1]
        if fault is not None:
            raise RedisConnectionError("injected fault")
        user_limit, source_limit, window, block = args
        user = self.state.bump((keys[0], keys[1]), (user_limit, window, block))
        if source_limit > 0 and user == 0:
            return [user, self.state.bump((keys[2], keys[3]), (source_limit, window, block))]
        return [user, 0]


class FakeRedis(RedisCache):
    def __init__(self, state: RedisState) -> None:
        self.state = state

    @override
    def async_register_script(self, script: str) -> Script:
        if not script:
            raise ValueError("the throttle registers a non-empty script")
        return Script(self.state)

    @override
    async def async_delete_cache(self, key: str) -> bool:
        await asyncio.sleep(0)
        if self.state.faults.get("delete") is not None:
            raise RedisConnectionError("injected fault")
        self.state.counters.pop(key, None)
        return True


def make_cache(spec: RedisInput | None, clock: Clock) -> RedisCache | None:
    if spec is None:
        return None
    faults = spec.get("faults")
    if faults is None:
        faults = {}
    return FakeRedis(RedisState(clock, faults))


class Session:
    def __init__(self, spec: RedisInput | None) -> None:
        self.clock = Clock(0.0)
        self.counters = InMemoryCache(
            max_size_in_memory=MAX_COUNTERS,
            default_ttl=DEFAULT_WINDOW,
            clock=self.clock,
        )
        self.blocks = InMemoryCache(
            max_size_in_memory=MAX_BLOCKS,
            default_ttl=DEFAULT_BLOCK,
            clock=self.clock,
        )
        self.cache = make_cache(spec, self.clock)
