import asyncio
import logging
import os
import time
from dataclasses import dataclass
from typing import Final, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.proxy.auth import handle_jwt
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

litellm.suppress_debug_info = True

PUBLIC_KEY_URL_VARIABLE: Final = "JWT_PUBLIC_KEY_URL"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TTL: Final[TypeAdapter[float]] = TypeAdapter(float)
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


# the one method get_public_key uses, typed so the call does not carry LiteLLM's bare dict
class KeyLookup(Protocol):
    async def get_public_key(self, kid: str | None) -> JsonValue: ...


class Clock(Protocol):
    def time(self) -> float: ...

    async def sleep(self, delay: float) -> None: ...


class FakeClock(Clock):
    def __init__(self) -> None:
        self.now: float = 0.0
        self.sleeps: list[float] = []

    def time(self) -> float:
        return self.now

    async def sleep(self, delay: float) -> None:
        self.sleeps.append(float(delay))


# the four levels LiteLLM logs on, where only warning is compared: the rest stay silent
class Logger:
    def debug(self, message: str, *args: LogArgument) -> None: ...

    def info(self, message: str, *args: LogArgument) -> None: ...

    def warning(self, message: str, *args: LogArgument) -> None: ...

    def error(self, message: str, *args: LogArgument) -> None: ...


class Recorder(Logger):
    def __init__(self) -> None:
        self.warnings: list[str] = []

    def warning(self, message: str, *args: LogArgument) -> None:
        del message, args
        self.warnings.append("warning")


# LiteLLM's in-memory tier, narrowed to the expiry rule the resolver relies on
class Cache(InMemoryCache):
    def __init__(self, clock: FakeClock) -> None:
        super().__init__()
        self.clock = clock
        self.values: dict[str, JsonValue] = {}
        self.expiries: dict[str, float] = {}
        self.writes: list[WriteView] = []

    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        del kwargs
        expires_at = self.expiries.get(key)
        if expires_at is not None and self.clock.time() <= expires_at:
            return self.values.get(key)
        self.expiries.pop(key, None)
        self.values.pop(key, None)
        return None

    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        seconds = TTL.validate_python(kwargs["ttl"])
        expires_at = self.expiries.get(key)
        if expires_at is None or self.clock.time() > expires_at:
            expires_at = self.clock.time() + seconds
        self.values[key] = value
        self.expiries[key] = expires_at
        self.writes.append(WriteView(key=key, ttl=seconds))


class ProxyFailure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    code: str
    type: str
    param: str | None


class ScriptedTransport:
    def __init__(self) -> None:
        self.replies: list[Reply] = []
        self.requests: list[str] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(str(request.url))
        reply = self.replies.pop(0) if self.replies else Reply(kind="transport")
        if reply["kind"] == "transport":
            raise httpx.ConnectError("the scripted provider is unreachable", request=request)
        return httpx.Response(reply["status"], text=reply["body"])


# the stage answers a bare key failure as a 401 and an outage as the retryable 503
def error_view(error: BaseException) -> ErrorView:
    try:
        proxy = ProxyFailure.model_validate(error)
    except ValidationError:
        return ErrorView(status=401, type="auth_error", param="None", code="401")
    return ErrorView(
        status=int(proxy.code) if proxy.code.isdigit() else 0,
        type=proxy.type,
        param=proxy.param,
        code=proxy.code,
    )


def set_public_key_url(url: str | None) -> None:
    if url is None:
        os.environ.pop(PUBLIC_KEY_URL_VARIABLE, None)
    else:
        os.environ[PUBLIC_KEY_URL_VARIABLE] = url


async def build_handler(
    settings: Settings, cache: Cache, transport: ScriptedTransport
) -> JWTHandler:
    litellm_jwtauth = LiteLLM_JWTAuth(
        public_key_ttl=settings["public_key_ttl"],
        public_key_stale_ttl=settings["public_key_stale_ttl"],
    )
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(in_memory_cache=cache),
        litellm_jwtauth=litellm_jwtauth,
    )
    handler.http_handler.client = httpx.AsyncClient(
        transport=httpx.MockTransport(transport.handle)
    )
    return handler


def outcome_of(task: asyncio.Future[JsonValue]) -> tuple[JsonValue, ErrorView | None]:
    failure = task.exception()
    if failure is not None:
        return (None, error_view(failure))
    return (JSON.validate_python(task.result()), None)


@dataclass
class Session:
    lookup: KeyLookup
    clock: FakeClock
    transport: ScriptedTransport
    cache: Cache
    recorder: Recorder

    async def step(self, step: Step) -> StepResult:
        self.clock.now = step["now"]
        scheduled = step.get("http")
        if scheduled is not None:
            self.transport.replies.extend(scheduled)
        requests = len(self.transport.requests)
        sleeps = len(self.clock.sleeps)
        writes = len(self.cache.writes)
        warnings = len(self.recorder.warnings)
        kid = step.get("kid")
        if step.get("concurrent"):
            first = asyncio.ensure_future(self.lookup.get_public_key(kid))
            second = asyncio.ensure_future(self.lookup.get_public_key(kid))
            await asyncio.wait({first, second})
            task = first if first.exception() is not None else second
        else:
            task = asyncio.ensure_future(self.lookup.get_public_key(kid))
            await asyncio.wait({task})
        value, error = outcome_of(task)
        return StepResult(
            value=value,
            error=error,
            requests=self.transport.requests[requests:],
            sleeps=self.clock.sleeps[sleeps:],
            writes=self.cache.writes[writes:],
            warnings=self.recorder.warnings[warnings:],
        )


async def scenario_answer(
    entry: Scenario, clock: FakeClock, recorder: Recorder
) -> ScenarioResult:
    set_public_key_url(entry["settings"]["public_key_url"])
    transport = ScriptedTransport()
    cache = Cache(clock)
    try:
        handler = await build_handler(entry["settings"], cache, transport)
    except ValidationError:
        return ScenarioResult(settings_error=True, steps=[])
    session = Session(handler, clock, transport, cache, recorder)
    steps = [await session.step(step) for step in entry["steps"]]
    return ScenarioResult(settings_error=False, steps=steps)


async def answer(input_value: Input) -> Answer:
    clock = FakeClock()
    recorder = Recorder()
    object.__setattr__(handle_jwt, "time", clock)
    object.__setattr__(handle_jwt, "verbose_proxy_logger", recorder)
    original_sleep = asyncio.sleep
    object.__setattr__(asyncio, "sleep", clock.sleep)
    try:
        scenarios = [
            await scenario_answer(entry, clock, recorder)
            for entry in input_value["scenarios"]
        ]
    finally:
        object.__setattr__(asyncio, "sleep", original_sleep)
        object.__setattr__(handle_jwt, "verbose_proxy_logger", logging.getLogger("LiteLLM Proxy"))
        object.__setattr__(handle_jwt, "time", time)
        os.environ.pop(PUBLIC_KEY_URL_VARIABLE, None)
    return Answer(scenarios=scenarios)


def run(input_value: Input) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(answer(input_value))
