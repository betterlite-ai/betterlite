import asyncio
import logging
import time
from dataclasses import replace
from typing import Final, NotRequired, TypedDict

from fastapi import Request
from litellm.proxy._types import ProxyException
from litellm.proxy.auth.login_throttle import LoginAttempt, LoginThrottle
from pydantic import JsonValue, ValidationError
from starlette.types import Scope

from login_throttle.fakes import CaptureHandler, RedisInput, Session

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


def classify(message: str) -> str:
    for needle, tag in WARNING_TOKENS:
        if needle in message:
            return tag
    return f"other:{message}"


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
    warnings = [classify(message) for message in handler.drain()]
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
