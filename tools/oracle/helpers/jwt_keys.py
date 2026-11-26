import asyncio
import logging
import os
import time
from collections.abc import Awaitable
from typing import Final, NotRequired, Protocol, TypedDict, TypeVar
from unittest.mock import patch

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy._types import LiteLLM_JWTAuth, ProxyException
from litellm.proxy.auth import handle_jwt
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

litellm.suppress_debug_info = True

PUBLIC_KEY_URL_VARIABLE: Final = "JWT_PUBLIC_KEY_URL"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
T = TypeVar("T")
LogArgument = str | int | float | BaseException


class Settings(TypedDict):
    public_key_url: str | None
    public_key_ttl: float
    public_key_stale_ttl: float


class Reply(TypedDict):
    kind: str
    status: NotRequired[int]
    body: NotRequired[str]


class Step(TypedDict):
    now: float
    kid: NotRequired[str | None]
    http: NotRequired[list[Reply]]
    concurrent: NotRequired[bool]


class Scenario(TypedDict):
    settings: Settings
    steps: list[Step]


class Input(TypedDict):
    scenarios: list[Scenario]


class ErrorView(TypedDict):
    status: int
    type: str
    param: str | None
    code: str


class WriteView(TypedDict):
    key: str
    ttl: float


class StepResult(TypedDict):
    value: JsonValue
    error: ErrorView | None
    requests: list[str]
    sleeps: list[float]
    writes: list[WriteView]
    warnings: list[str]


class ScenarioResult(TypedDict):
    settings_error: bool
    steps: list[StepResult]


class Answer(TypedDict):
    scenarios: list[ScenarioResult]


# the one method get_public_key uses from the handler, typed so the call does not carry LiteLLM's bare dict
class KeyLookup(Protocol):
    async def get_public_key(self, kid: str | None) -> object: ...


class FakeClock:
    def __init__(self) -> None:
        self.now: float = 0.0
        self.sleeps: list[float] = []

    def time(self) -> float:
        return self.now

    async def sleep(self, delay: float) -> None:
        self.sleeps.append(float(delay))


class Recorder:
    def __init__(self) -> None:
        self.warnings: list[str] = []

    def debug(self, message: str, *args: LogArgument) -> None:
        del message, args

    def info(self, message: str, *args: LogArgument) -> None:
        del message, args

    def warning(self, message: str, *args: LogArgument) -> None:
        del message, args
        self.warnings.append("warning")

    def error(self, message: str, *args: LogArgument) -> None:
        del message, args


# LiteLLM's in-memory tier, narrowed to the expiry rule the resolver relies on: a read lives until the clock
# passes its expiry, and a second write keeps an unexpired expiry while recording the ttl it was asked for.
class Cache(InMemoryCache):
    def __init__(self, clock: FakeClock) -> None:
        super().__init__(clock=clock.time)
        self.clock = clock
        self.values: dict[str, JsonValue] = {}
        self.expiries: dict[str, float] = {}
        self.writes: list[WriteView] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        del kwargs
        expires_at = self.expiries.get(key)
        if expires_at is None:
            return None
        if self.clock.time() > expires_at:
            del self.expiries[key]
            self.values.pop(key, None)
            return None
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        ttl = kwargs.get("ttl")
        if not isinstance(ttl, (int, float)) or isinstance(ttl, bool):
            raise ValueError("a JWKS cache write carries a numeric ttl")
        seconds = float(ttl)
        expires_at = self.expiries.get(key)
        if expires_at is None or self.clock.time() > expires_at:
            expires_at = self.clock.time() + seconds
        self.values[key] = value
        self.expiries[key] = expires_at
        self.writes.append(WriteView(key=key, ttl=seconds))


class ScriptedTransport:
    def __init__(self) -> None:
        self.replies: list[Reply] = []
        self.requests: list[str] = []

    def script(self, replies: list[Reply]) -> None:
        self.replies.extend(replies)

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(str(request.url))
        if not self.replies:
            raise httpx.ConnectError("the scripted provider is unreachable", request=request)
        reply = self.replies.pop(0)
        if reply["kind"] == "transport":
            raise httpx.ConnectError("the scripted provider is unreachable", request=request)
        return httpx.Response(reply.get("status", 0), text=reply.get("body", ""))


async def attempt(call: Awaitable[T]) -> T | BaseException:
    [outcome] = await asyncio.gather(call, return_exceptions=True)
    return outcome


def status_code(code: str) -> int:
    return int(code) if code.isdigit() else 0


# The stage answers a bare key failure as Authentication Error 401 and an outage as the retryable 503, the same
# mapping betterlite's own error type applies; the compared fields are the ones the denial carries.
def error_view(error: BaseException) -> ErrorView:
    if isinstance(error, ProxyException):
        return ErrorView(
            status=status_code(error.code),
            type=error.type,
            param=error.param,
            code=error.code,
        )
    return ErrorView(status=401, type="auth_error", param="None", code="401")


def set_public_key_url(url: str | None) -> None:
    if url is None:
        os.environ.pop(PUBLIC_KEY_URL_VARIABLE, None)
    else:
        os.environ[PUBLIC_KEY_URL_VARIABLE] = url


async def build_handler(settings: Settings, cache: Cache, transport: ScriptedTransport) -> JWTHandler:
    litellm_jwtauth = LiteLLM_JWTAuth(
        public_key_ttl=settings["public_key_ttl"],
        public_key_stale_ttl=settings["public_key_stale_ttl"],
    )
    handler = JWTHandler()
    await handler.http_handler.close()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(in_memory_cache=cache),
        litellm_jwtauth=litellm_jwtauth,
    )
    handler.http_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(transport.handle)
    )
    return handler


class Session:
    def __init__(
        self,
        lookup: KeyLookup,
        clock: FakeClock,
        transport: ScriptedTransport,
        cache: Cache,
        recorder: Recorder,
    ) -> None:
        self.lookup = lookup
        self.clock = clock
        self.transport = transport
        self.cache = cache
        self.recorder = recorder

    async def step(self, step: Step) -> StepResult:
        self.clock.now = step["now"]
        self.transport.script(step.get("http", []))
        requests = len(self.transport.requests)
        sleeps = len(self.clock.sleeps)
        writes = len(self.cache.writes)
        warnings = len(self.recorder.warnings)
        kid = step.get("kid")
        outcome: object | BaseException
        if step.get("concurrent", False):
            first, second = await asyncio.gather(
                self.lookup.get_public_key(kid),
                self.lookup.get_public_key(kid),
                return_exceptions=True,
            )
            outcome = first if isinstance(first, BaseException) else second
        else:
            outcome = await attempt(self.lookup.get_public_key(kid))
        value: JsonValue = None
        error: ErrorView | None = None
        if isinstance(outcome, BaseException):
            error = error_view(outcome)
        else:
            value = JSON.validate_python(outcome)
        return StepResult(
            value=value,
            error=error,
            requests=self.transport.requests[requests:],
            sleeps=self.clock.sleeps[sleeps:],
            writes=self.cache.writes[writes:],
            warnings=self.recorder.warnings[warnings:],
        )


async def scenario_answer(entry: Scenario, clock: FakeClock, recorder: Recorder) -> ScenarioResult:
    settings = entry["settings"]
    set_public_key_url(settings["public_key_url"])
    transport = ScriptedTransport()
    cache = Cache(clock)
    try:
        handler = await build_handler(settings, cache, transport)
    except ValidationError:
        return ScenarioResult(settings_error=True, steps=[])
    session = Session(handler, clock, transport, cache, recorder)
    try:
        steps = [await session.step(step) for step in entry["steps"]]
    finally:
        await handler.http_handler.close()
    return ScenarioResult(settings_error=False, steps=steps)


async def answer(input_value: Input) -> Answer:
    clock = FakeClock()
    recorder = Recorder()
    object.__setattr__(handle_jwt, "time", clock)
    object.__setattr__(handle_jwt, "verbose_proxy_logger", recorder)
    try:
        with patch.object(asyncio, "sleep", clock.sleep):
            scenarios = [
                await scenario_answer(entry, clock, recorder)
                for entry in input_value["scenarios"]
            ]
    finally:
        object.__setattr__(handle_jwt, "verbose_proxy_logger", logging.getLogger("LiteLLM Proxy"))
        object.__setattr__(handle_jwt, "time", time)
        os.environ.pop(PUBLIC_KEY_URL_VARIABLE, None)
    return Answer(scenarios=scenarios)


def run(input_value: Input) -> Answer:
    return asyncio.run(answer(input_value))
