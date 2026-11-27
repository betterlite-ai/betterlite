import asyncio
import logging
import time
from collections.abc import Mapping, Sequence
from dataclasses import replace
from typing import Final, NotRequired, TypedDict

from fastapi import Request
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.constants import (
    LOGIN_THROTTLE_MAX_TRACKED_BLOCKS as MAX_BLOCKS,
    LOGIN_THROTTLE_MAX_TRACKED_COUNTERS as MAX_COUNTERS,
)
from litellm.proxy._types import ProxyException
from litellm.proxy.auth.login_throttle import (
    DEFAULT_FAILED_LOGIN_BLOCK_SECONDS as DEFAULT_BLOCK,
    DEFAULT_FAILED_LOGIN_WINDOW_SECONDS as DEFAULT_WINDOW,
    LoginAttempt,
    LoginThrottle,
)
from pydantic import JsonValue, TypeAdapter, ValidationError
from redis.exceptions import ConnectionError as RedisConnectionError
from starlette.types import Scope
from typing_extensions import override

MALFORMED: Final = "malformed"
EXPIRED: Final = -2
NO_EXPIRY: Final = -1.0
TTL_KEYS: Final = TypeAdapter(tuple[str, str, str, str])
SCRIPT_ARGS: Final = TypeAdapter(tuple[int, ...])

WARNING_TOKENS: Final[tuple[tuple[str, str], ...]] = (
    ("Redis failed while counting", "redis_fallback"),
    ("Admin UI sign-in refused", "refused"),
    ("Admin UI sign-in blocked", "blocked"),
    ("counted per worker", "counters_per_worker"),
    ("is not set or not a valid list of ranges", "source_limit_off"),
    ("expected a mapping of address or range to limit", "invalid_overrides_mapping"),
    ("max_failed_login_attempts_per_source_overrides", "invalid_override_value"),
    ("Invalid trusted_proxy_ranges value", "invalid_topology_shape"),
    ("Invalid address or range", "invalid_network"),
    ("max_failed_login_attempts_per_source", "invalid_source_limit"),
    ("failed_login_window_seconds", "invalid_window"),
    ("failed_login_block_seconds", "invalid_block"),
)


class RequestInput(TypedDict):
    peer: NotRequired[str]
    x_forwarded_for: NotRequired[str]
    host: NotRequired[str]


class StepInput(TypedDict):
    kind: str
    username: NotRequired[str]


class PhaseInput(TypedDict):
    request: RequestInput
    now: float
    steps: list[StepInput]


class ConfigInput(TypedDict):
    general_settings: dict[str, JsonValue]


class RedisInput(TypedDict):
    faults: NotRequired[dict[str, str]]


class Input(TypedDict):
    config: ConfigInput
    phases: list[PhaseInput]
    redis: NotRequired[RedisInput]


class ErrorOutput(TypedDict):
    code: str
    type: str
    param: str | None
    message: str
    headers: dict[str, str]


class StepOutput(TypedDict):
    step: str
    outcome: str
    error: ErrorOutput | None
    user_block: int | None
    source_block: int | None


class ThrottleOutput(TypedDict):
    client_ip: str
    source_limit: int | None
    user_limit: int
    window_seconds: int
    block_seconds: int
    enabled: bool


class PhaseOutput(TypedDict):
    throttle: ThrottleOutput
    steps: list[StepOutput]
    warnings: list[str]


class Output(TypedDict):
    phases: list[PhaseOutput]


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
        self.cache: RedisCache | None = None
        if spec is not None:
            faults: dict[str, str] | None = spec.get("faults")
            if faults is None:
                faults = {}
            self.cache = FakeRedis(RedisState(self.clock, faults))


def build_request(spec: RequestInput) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    host = spec.get("host")
    if host is not None:
        headers.append((b"host", host.encode()))
    forwarded = spec.get("x_forwarded_for")
    if forwarded is not None:
        headers.append((b"x-forwarded-for", forwarded.encode()))
    peer = spec.get("peer")
    client = (peer, 0) if peer is not None else None
    scope: Scope = {"type": "http", "path": "/login", "headers": headers, "client": client}
    return Request(scope=scope)


async def attempt_step(
    throttle: LoginThrottle, username: str, view: StepOutput
) -> tuple[LoginAttempt | None, StepOutput]:
    try:
        attempt = await throttle.attempt(username)
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        view["outcome"] = "refused"
        view["error"] = ErrorOutput(
            code=error.code, type=error.type, param=error.param,
            message=error.message, headers=error.headers,
        )
        return None, view
    except ValidationError:
        view["outcome"] = "error"
        return None, view
    view["outcome"] = "allowed"
    return attempt, view


async def settle_step(
    pending: LoginAttempt | None, kind: str, view: StepOutput
) -> tuple[LoginAttempt | None, StepOutput]:
    if pending is None:
        view["outcome"] = "no_attempt"
        return None, view
    try:
        if kind == "failed":
            await pending.failed()
        else:
            await pending.succeeded()
    except ValidationError:
        view["outcome"] = "error"
        return None, view
    view["outcome"] = "ok"
    return None, view


async def record_step(throttle: LoginThrottle, username: str, view: StepOutput) -> StepOutput:
    try:
        user_block, source_block = await throttle.record_failure(username)
    except ValidationError:
        view["outcome"] = "error"
        return view
    view["outcome"] = "ok"
    view["user_block"] = user_block
    view["source_block"] = source_block
    return view


async def clear_step(throttle: LoginThrottle, username: str, view: StepOutput) -> StepOutput:
    try:
        await throttle.clear_pair(username)
    except ValidationError:
        view["outcome"] = "error"
        return view
    view["outcome"] = "ok"
    return view


async def run_step(
    throttle: LoginThrottle, step: StepInput, pending: LoginAttempt | None
) -> tuple[LoginAttempt | None, StepOutput]:
    kind = step["kind"]
    username = step.get("username")
    view: StepOutput = {
        "step": kind,
        "outcome": "unknown",
        "error": None,
        "user_block": None,
        "source_block": None,
    }
    if kind == "attempt":
        return await attempt_step(throttle, "" if username is None else username, view)
    if kind in ("failed", "succeeded"):
        return await settle_step(pending, kind, view)
    if kind == "record_failure":
        return pending, await record_step(throttle, "" if username is None else username, view)
    if kind == "clear_pair":
        return pending, await clear_step(throttle, "" if username is None else username, view)
    return pending, view


async def finish_phase(
    session: Session, settings: dict[str, JsonValue], phase: PhaseInput, handler: CaptureHandler
) -> PhaseOutput:
    request = build_request(phase["request"])
    throttle = replace(
        LoginThrottle.from_request(request, settings, session.cache),
        counters=session.counters,
        blocks=session.blocks,
    )
    steps: list[StepOutput] = []
    pending: LoginAttempt | None = None
    for step in phase["steps"]:
        pending, view = await run_step(throttle, step, pending)
        steps.append(view)
    throttle_view: ThrottleOutput = {
        "client_ip": throttle.client_ip,
        "source_limit": throttle.source_limit,
        "user_limit": throttle.user_limit,
        "window_seconds": throttle.window_seconds,
        "block_seconds": throttle.block_seconds,
        "enabled": throttle.enabled,
    }
    warnings = [
        next((tag for needle, tag in WARNING_TOKENS if needle in message), f"other:{message}")
        for message in handler.drain()
    ]
    return {"throttle": throttle_view, "steps": steps, "warnings": warnings}


async def run_phase(
    phase: PhaseInput, settings: dict[str, JsonValue], handler: CaptureHandler, session: Session
) -> PhaseOutput:
    session.clock.now = phase["now"]
    saved = time.time
    time.time = session.clock
    try:
        return await finish_phase(session, settings, phase, handler)
    finally:
        time.time = saved


async def run_phases(input_value: Input, handler: CaptureHandler) -> Output:
    session = Session(input_value.get("redis"))
    settings = input_value["config"]["general_settings"]
    phases = [await run_phase(phase, settings, handler, session) for phase in input_value["phases"]]
    return {"phases": phases}


def run(input_value: Input) -> Output:
    handler = CaptureHandler()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(handler)
    logger.setLevel(logging.WARNING)
    try:
        return asyncio.run(run_phases(input_value, handler))
    finally:
        logger.removeHandler(handler)
