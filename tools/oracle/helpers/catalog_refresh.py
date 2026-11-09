import asyncio
import os
import random
import threading
from collections.abc import Awaitable, Callable, Coroutine
from decimal import Decimal
from typing import NamedTuple, Protocol, TypeAlias, cast

import httpx
import litellm
import litellm.litellm_core_utils.get_model_cost_map as cost_map
from litellm.litellm_core_utils.fallback_generalizations import match_routing_generalization
from litellm.litellm_core_utils.get_model_cost_map import (
    CostMapSourceInfo,
    ModelCostMapReloaded,
    ModelCostMapReloadUnavailable,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Headers: TypeAlias = dict[str, str]
RESERVED: frozenset[str] = frozenset({"sample_spec", "fallback_generalizations"})
LOCAL_ENV: str = "LITELLM_LOCAL_MODEL_COST_MAP"
MODEL_LIMIT: int = 50


class _LoadCostMap(Protocol):
    def __call__(  # noqa: PLR0913
        self,
        *,
        url: str,
        timeout: int,
        max_attempts: int,
        sleep: Callable[[float], None],
        rng: random.Random | None,
        client: object,
    ) -> dict[str, Json]: ...


class _RefetchCostMap(Protocol):
    def __call__(  # noqa: PLR0913
        self,
        *,
        url: str,
        timeout: int,
        max_attempts: int,
        sleep: Callable[[float], Awaitable[None]],
        rng: random.Random | None,
        client: object,
    ) -> Coroutine[object, object, ModelCostMapReloaded | ModelCostMapReloadUnavailable]: ...


class _AdoptCostMap(Protocol):
    def __call__(self, new_model_cost_map: dict[str, Json]) -> int: ...


load_model_cost_map = cast(_LoadCostMap, cost_map.get_model_cost_map)
refetch_model_cost_map = cast(_RefetchCostMap, cost_map.refetch_model_cost_map)
adopt_model_cost_map = cast(_AdoptCostMap, cost_map.adopt_model_cost_map)

class _Reply(NamedTuple):
    status: int
    headers: Headers
    body: str
    error: str | None


class _Request(NamedTuple):
    url: str
    replies: list[_Reply]
    max_attempts: int
    timeout: int
    jitter: list[float]


class _Facts(NamedTuple):
    kind: str
    reason: str | None
    immediate: CostMapSourceInfo
    final: CostMapSourceInfo
    requests: int
    sleeps: list[float]


class _SleepLog:
    def __init__(self) -> None:
        self.values: list[float] = []
        self._lock = threading.Lock()

    def record(self, seconds: float) -> None:
        with self._lock:
            self.values.append(seconds)


class _GatedSleeper:
    def __init__(self, log: _SleepLog) -> None:
        self._log = log
        self.release = threading.Event()

    def __call__(self, seconds: float) -> None:
        self._log.record(seconds)
        self.release.wait(timeout=10)


class _AsyncSleeper:
    def __init__(self, log: _SleepLog) -> None:
        self._log = log

    async def sleep(self, seconds: float) -> None:
        self._log.record(seconds)


class _Replies:
    def __init__(self, replies: list[_Reply]) -> None:
        self._replies = replies
        self._lock = threading.Lock()
        self.requests = 0

    def next(self) -> httpx.Response:
        with self._lock:
            index = self.requests
            self.requests += 1
            reply = (
                self._replies[index]
                if index < len(self._replies)
                else _Reply(0, {}, "", "unscripted request")
            )
        if reply.error == "invalid_url":
            raise httpx.InvalidURL("unsupported url")
        if reply.error is not None:
            raise httpx.ConnectError("connection failed")
        return httpx.Response(
            status_code=reply.status,
            headers=reply.headers,
            content=reply.body.encode(),
        )


class _SyncClient:
    def __init__(self, replies: _Replies) -> None:
        self._replies = replies

    def get(self, url: str, *, timeout: float | None = None) -> httpx.Response:
        del url, timeout
        return self._replies.next()


class _AsyncClient:
    def __init__(self, replies: _Replies) -> None:
        self._replies = replies

    async def get(
        self,
        url: str,
        *,
        timeout: float | None = None,  # noqa: ASYNC109
    ) -> httpx.Response:
        del url, timeout
        return self._replies.next()


class _Jitter:
    def __init__(self, values: list[float]) -> None:
        self.values = values

    def uniform(self, lower: float, upper: float) -> float:
        del lower, upper
        return self.values.pop(0) if self.values else 0.0


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else string_value(value, name)


def _int(input_value: dict[str, Json], name: str, default: int) -> int:
    value = input_value.get(name)
    if value is None:
        return default
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _strings(input_value: dict[str, Json], name: str) -> list[str]:
    value = input_value.get(name, [])
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [string_value(entry, name) for entry in value]


def _reply_inputs(input_value: dict[str, Json]) -> list[_Reply]:
    replies = input_value.get("replies", [])
    if not isinstance(replies, list):
        raise ValueError("replies must be a list")
    parsed: list[_Reply] = []
    for entry in replies:
        reply = object_value(entry, "reply")
        headers = reply.get("headers", {})
        parsed.append(
            _Reply(
                status=_int(reply, "status", 0),
                headers={
                    name: string_value(value, f"headers.{name}")
                    for name, value in object_value(headers, "headers").items()
                },
                body=_optional_string(reply, "body") or "",
                error=_optional_string(reply, "error"),
            )
        )
    return parsed


def _request(input_value: dict[str, Json]) -> _Request:
    return _Request(
        url=litellm.model_cost_map_url,
        replies=_reply_inputs(input_value),
        max_attempts=_int(input_value, "max_attempts", 3),
        timeout=_int(input_value, "timeout", 5),
        jitter=[float(value) for value in _strings(input_value, "jitter")],
    )


def _apply_environment(environment: dict[str, Json]) -> dict[str, str | None]:
    saved = {LOCAL_ENV: os.environ.get(LOCAL_ENV)}
    configured = environment.get(LOCAL_ENV)
    if configured is None:
        os.environ.pop(LOCAL_ENV, None)
    else:
        os.environ[LOCAL_ENV] = string_value(configured, LOCAL_ENV)
    return saved


def _restore_environment(saved: dict[str, str | None]) -> None:
    for name, value in saved.items():
        if value is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = value


def _source_info() -> CostMapSourceInfo:
    return cost_map.get_model_cost_map_source_info()


def _join_retry_threads(before: set[threading.Thread]) -> None:
    for thread in threading.enumerate():
        if thread not in before:
            thread.join(timeout=10)


def _load(url: str, request: _Request, replies: _Replies, log: _SleepLog) -> CostMapSourceInfo:
    sleeper = _GatedSleeper(log)
    before = set(threading.enumerate())
    model_cost = load_model_cost_map(
        url=url,
        timeout=request.timeout,
        max_attempts=request.max_attempts,
        sleep=sleeper,
        rng=cast(random.Random, _Jitter(list(request.jitter))),
        client=_SyncClient(replies),
    )
    litellm.model_cost = model_cost
    immediate = _source_info()
    sleeper.release.set()
    _join_retry_threads(before)
    return immediate


def _startup(request: _Request) -> _Facts:
    replies = _Replies(request.replies)
    log = _SleepLog()
    immediate = _load(request.url, request, replies, log)
    return _Facts("startup", None, immediate, _source_info(), replies.requests, log.values)


def _reload(request: _Request) -> _Facts:
    replies = _Replies(request.replies)
    log = _SleepLog()
    immediate = _load(request.url, request, replies, log)
    result = asyncio.run(
        refetch_model_cost_map(
            url=request.url,
            timeout=request.timeout,
            max_attempts=request.max_attempts,
            sleep=_AsyncSleeper(log).sleep,
            rng=cast(random.Random, _Jitter(list(request.jitter))),
            client=_AsyncClient(replies),
        )
    )
    if isinstance(result, ModelCostMapReloaded):
        adopt_model_cost_map(cast(dict[str, Json], result.model_cost_map))
        return _Facts("reloaded", None, immediate, _source_info(), replies.requests, log.values)
    return _Facts(
        "unavailable",
        result.reason,
        immediate,
        _source_info(),
        replies.requests,
        log.values,
    )


def _seconds(value: float) -> str:
    return format(Decimal(str(value)).normalize(), "f")


def _fallback_kind(reason: str) -> str:
    if reason == "Remote data failed integrity validation":
        return "integrity"
    if reason.startswith("Remote fetch failed:"):
        return "fetch"
    if reason.startswith("Remote data failed to load"):
        return "load"
    return "other"


def _reason_kind(reason: str) -> str:
    if reason.startswith("HTTP "):
        return "http"
    if reason.startswith("invalid JSON"):
        return "invalid_json"
    if "integrity validation" in reason:
        return "integrity"
    if "failed to load" in reason:
        return "load"
    return "other"


def _provenance_facts(info: CostMapSourceInfo) -> Json:
    reason = info.get("fallback_reason")
    fallback: Json = None
    if isinstance(reason, str):
        fallback = _fallback_kind(reason)
    return {
        "source": info.get("source"),
        "url": info.get("url"),
        "is_env_forced": info.get("is_env_forced"),
        "fallback": fallback,
        "loaded_at_set": info.get("loaded_at") is not None,
        "revision": info.get("source_revision"),
        "etag": info.get("etag"),
    }


def _model_cost() -> dict[str, Json]:
    return cast(dict[str, Json], litellm.model_cost)


def _catalog_facts() -> Json:
    model_cost = _model_cost()
    count = len(model_cost)
    names: list[Json] = []
    names.extend(sorted(model_cost))
    models: Json = None if count > MODEL_LIMIT else names
    return {"count": count, "models": models}


def _probe_facts(probe_models: list[str]) -> Json:
    model_cost = _model_cost()
    probes: dict[str, Json] = {}
    for name in probe_models:
        entry = model_cost.get(name)
        provider: Json = None
        if isinstance(entry, dict):
            value = entry.get("litellm_provider")
            if isinstance(value, str):
                provider = value
        probes[name] = provider
    return probes


def _generalization(probe: str | None) -> Json:
    if probe is None:
        return None
    return match_routing_generalization(probe)


def _facts_json(input_value: dict[str, Json], facts: _Facts) -> Json:
    return {
        "kind": facts.kind,
        "reason_kind": None if facts.reason is None else _reason_kind(facts.reason),
        "before": _provenance_facts(facts.immediate),
        "after": _provenance_facts(facts.final),
        "catalog": _catalog_facts(),
        "probes": _probe_facts(_strings(input_value, "probe_models")),
        "generalization": _generalization(_optional_string(input_value, "probe_generalization")),
        "requests": facts.requests,
        "sleeps": [_seconds(value) for value in facts.sleeps],
    }


def run(input_value: dict[str, Json]) -> Json:
    request = _request(input_value)
    environment = object_value(input_value.get("environment", {}), "environment")
    saved = _apply_environment(environment)
    try:
        operation = string_value(input_value.get("operation"), "operation")
        facts = _startup(request) if operation == "startup" else _reload(request)
    finally:
        _restore_environment(saved)
    return _facts_json(input_value, facts)
