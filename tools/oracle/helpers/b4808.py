import asyncio
import copy
import json
import re
import sys
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from http import HTTPStatus
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import httpx
import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PACKAGE = "litellm.a2a_protocol.providers.watsonx_orchestrate"
_COUNTER_ID = re.compile(r"^00000000-0000-0000-0000-[0-9a-f]{12}$")
_PROVIDER_DIR = Path(litellm.__file__).parent / "a2a_protocol" / "providers" / "watsonx_orchestrate"
_PROVIDERS_DIR = _PROVIDER_DIR.parent


def _load(name: str, path: Path) -> ModuleType:
    spec = spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load LiteLLM module at {path}")
    module = module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _stub(name: str) -> None:
    # the a2a_protocol package needs the a2a SDK, so its providers load by path
    package = ModuleType(name)
    package.__path__ = []
    sys.modules[name] = package


for _name in ("litellm.a2a_protocol", "litellm.a2a_protocol.providers", _PACKAGE):
    _stub(_name)

_load("litellm.a2a_protocol.providers.base", _PROVIDERS_DIR / "base.py")
_TRANSFORMATION_MODULE = _load(f"{_PACKAGE}.transformation", _PROVIDER_DIR / "transformation.py")
_HANDLER_MODULE = _load(f"{_PACKAGE}.handler", _PROVIDER_DIR / "handler.py")
_CONFIG_MODULE = _load(f"{_PACKAGE}.config", _PROVIDER_DIR / "config.py")

_TOKEN_CACHE = cast(dict[str, object], vars(_HANDLER_MODULE)["_token_cache"])


class Provider(Protocol):
    handle_non_streaming: Callable[..., Awaitable[Json]]
    handle_streaming: Callable[..., AsyncIterator[dict[str, Json]]]


_PROVIDER = cast(Provider, vars(_CONFIG_MODULE)["WatsonxOrchestrateA2AConfig"]())


class SleepRecorder:
    def __init__(self) -> None:
        self.steps: list[dict[str, Json]] = []
        self.sleeps: list[Json] = []

    async def sleep(self, seconds: float) -> None:
        milliseconds = round(seconds * 1000)
        if not self.steps:
            self.sleeps.append(milliseconds)
            return
        last = self.steps[-1]
        last["delay_ms"] = cast(int, last["delay_ms"]) + milliseconds


class FakeResponse:
    def __init__(self, step: Mapping[str, Json], url: str) -> None:
        self._step = step
        self._url = url
        self.status = cast(int, step.get("status", 200))
        self.headers = {"content-type": cast(str, step.get("content_type") or "")}

    def raise_for_status(self) -> None:
        if not HTTPStatus.OK <= self.status < HTTPStatus.MULTIPLE_CHOICES:
            raise httpx.HTTPStatusError(
                f"the runs API answered with HTTP status {self.status}",
                request=httpx.Request("POST", self._url),
                response=httpx.Response(self.status),
            )

    def json(self) -> Json:
        return self._step.get("body")

    async def aread(self) -> bytes:
        if self._step.get("fail") == "read":
            raise httpx.ReadError("the scripted body cannot be read")
        return json.dumps(self._step.get("body")).encode()

    async def aiter_lines(self) -> AsyncIterator[str]:
        if self._step.get("fail") == "read":
            raise httpx.ReadError("the scripted body cannot be read")
        for line in cast(list[str], self._step.get("lines", [])):
            yield line


class ScriptedClient:
    def __init__(self, script: list[Json], timeout: Json) -> None:
        self._script = script
        self._timeout = timeout
        self.requests: list[dict[str, Json]] = []

    async def post(self, url: str, **kwargs: Json) -> FakeResponse:
        return self._answer("POST", url, kwargs.get("json"), kwargs.get("headers"))

    async def get(self, url: str, **kwargs: Json) -> FakeResponse:
        return self._answer("GET", url, None, kwargs.get("headers"))

    def _answer(self, method: str, url: str, body: Json, headers: Json) -> FakeResponse:
        if "/v1/orchestrate/" in url:
            self.requests.append(
                {
                    "method": method,
                    "url": url,
                    "headers": dict(cast(Mapping[str, str], headers or {})),
                    "body": copy.deepcopy(body),
                    "timeout": self._timeout,
                }
            )
        if not self._script:
            raise RuntimeError("the scripted transport has no answer left")
        step = cast(Mapping[str, Json], self._script[0])
        if not step.get("repeat"):
            self._script.pop(0)
        if step.get("fail") == "transport":
            raise httpx.ConnectError("the scripted transport failed")
        return FakeResponse(step, url)


def _client_factory(
    script: list[Json], clients: list[ScriptedClient]
) -> Callable[..., ScriptedClient]:
    def build(**options: Json) -> ScriptedClient:
        params = cast(dict[str, Json], options.get("params") or {})
        client = ScriptedClient(script, params.get("timeout"))
        clients.append(client)
        return client

    return build


async def call_provider(input_value: Mapping[str, Json], recorder: SleepRecorder) -> Json:
    request_id = cast(str, input_value.get("request_id", "request"))
    params = input_value["params"]
    kwargs = cast(dict[str, Json], input_value.get("kwargs") or {})
    if input_value.get("mode") == "streaming":
        events = _PROVIDER.handle_streaming(request_id, params, None, **kwargs)
        async for event in events:
            recorder.steps.append({"event": event, "delay_ms": 0})
        return cast(Json, recorder.steps)
    return await _PROVIDER.handle_non_streaming(request_id, params, None, **kwargs)


_EXCEPTION_KINDS: tuple[tuple[type[BaseException], str], ...] = (
    (httpx.ReadError, "read"),
    (httpx.TransportError, "transport"),
    (httpx.HTTPStatusError, "status"),
    (TimeoutError, "timeout"),
    (json.JSONDecodeError, "decode"),
)


def _value_kind(message: str) -> str:
    if "litellm_params is required" in message:
        return "config_missing"
    if "run_id" in message:
        return "no_run_id"
    if "is required in litellm_params" in message:
        return "params_missing"
    return "other"


def classify(error: Exception) -> str:
    for error_type, kind in _EXCEPTION_KINDS:
        if isinstance(error, error_type):
            return kind
    message = str(error)
    if isinstance(error, ValueError):
        return _value_kind(message)
    if isinstance(error, RuntimeError) and "non-success status" in message:
        return "non_success"
    return "other"


def normalize(value: Json, seen: dict[str, str]) -> Json:
    if isinstance(value, str):
        if _COUNTER_ID.match(value):
            return seen.setdefault(value, f"uuid-{len(seen)}")
        return value
    if isinstance(value, list):
        return [normalize(item, seen) for item in value]
    if isinstance(value, dict):
        return {key: normalize(value[key], seen) for key in sorted(value)}
    return value


def run(input_value: dict[str, Json]) -> Json:
    script = cast(list[Json], copy.deepcopy(input_value.get("steps", [])))
    clients: list[ScriptedClient] = []
    recorder = SleepRecorder()
    ids = (UUID(int=value) for value in range(1, 100))
    fake_asyncio = SimpleNamespace(sleep=recorder.sleep, TimeoutError=TimeoutError)
    fake_time = SimpleNamespace(monotonic=lambda: 0.0, time=lambda: 1_700_000_000.0)
    with (
        patch.object(_HANDLER_MODULE, "get_async_httpx_client", _client_factory(script, clients)),
        patch.object(_HANDLER_MODULE, "asyncio", fake_asyncio),
        patch.object(_HANDLER_MODULE, "time", fake_time),
        patch.object(_TRANSFORMATION_MODULE, "asyncio", fake_asyncio),
        patch.object(_TRANSFORMATION_MODULE, "uuid4", side_effect=ids),
    ):
        _TOKEN_CACHE.clear()
        try:
            outcome: Json = {"ok": asyncio.run(call_provider(input_value, recorder))}
        except Exception as error:  # noqa: BLE001 - the wire names every helper exception
            outcome = {"error": {"kind": classify(error)}}
    requests = [request for client in clients for request in client.requests]
    result: dict[str, Json] = {
        "outcome": outcome,
        "requests": cast(Json, requests),
        "sleeps": cast(Json, recorder.sleeps),
    }
    return normalize(result, {})
