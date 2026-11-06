import asyncio
import copy
import re
import sys
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from http import HTTPStatus
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from types import ModuleType
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PROVIDERS = Path(litellm.__file__).parent / "a2a_protocol" / "providers"
_PACKAGE = "litellm.a2a_protocol.providers"
_COUNTER_ID = re.compile(r"^00000000-0000-0000-0000-[0-9a-f]{12}$")


def _load(name: str, path: Path) -> ModuleType:
    spec = spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load LiteLLM module at {path}")
    module = module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _stub_package(name: str) -> None:
    package = ModuleType(name)
    package.__path__ = []
    sys.modules[name] = package


# litellm.a2a_protocol needs the a2a SDK at import, so only the provider files are loaded
for _name in ("litellm.a2a_protocol", _PACKAGE, f"{_PACKAGE}.pydantic_ai_agents"):
    _stub_package(_name)
_load(f"{_PACKAGE}.base", _PROVIDERS / "base.py")
_AGENTS = _PROVIDERS / "pydantic_ai_agents"
_TRANSFORMATION = _load(
    f"{_PACKAGE}.pydantic_ai_agents.transformation", _AGENTS / "transformation.py"
)
_load(f"{_PACKAGE}.pydantic_ai_agents.handler", _AGENTS / "handler.py")
_CONFIG = _load(f"{_PACKAGE}.pydantic_ai_agents.config", _AGENTS / "config.py")


class Provider(Protocol):
    def handle_non_streaming(
        self, request_id: str, params: Json, api_base: str | None, **kwargs: Json
    ) -> Awaitable[Json]: ...

    def handle_streaming(
        self, request_id: str, params: Json, api_base: str | None, **kwargs: Json
    ) -> AsyncIterator[dict[str, Json]]: ...


_PROVIDER = cast(Callable[[], Provider], vars(_CONFIG)["PydanticAIProviderConfig"])()


class ScriptedResponse:
    def __init__(self, status: int, body: Json) -> None:
        self.status = status
        self.body = body

    def raise_for_status(self) -> None:
        if not HTTPStatus.OK <= self.status < HTTPStatus.MULTIPLE_CHOICES:
            raise RuntimeError(f"HTTP status {self.status}")

    def json(self) -> Json:
        return self.body


class ScriptedClient:
    def __init__(self, script: list[dict[str, Json]], timeout: Json) -> None:
        self.script = script
        self.timeout = timeout
        self.requests: list[dict[str, Json]] = []

    async def post(self, url: str, json: Json, headers: dict[str, str]) -> ScriptedResponse:
        self.requests.append(
            {
                "url": url,
                "body": copy.deepcopy(json),
                "headers": cast(Json, dict(headers)),
                "timeout": self.timeout,
            }
        )
        if not self.script:
            raise RuntimeError("the scripted transport has no response left")
        step = self.script.pop(0)
        if step.get("raises"):
            raise RuntimeError("the scripted transport failed")
        return ScriptedResponse(cast(int, step.get("status", 200)), step.get("body"))


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


def classify(error: Exception) -> dict[str, Json]:
    message = str(error)
    if isinstance(error, TimeoutError):
        return {"kind": "timeout", "message": message}
    if message.startswith("Task ") and "ended with state" in message:
        return {"kind": "task_ended", "message": message}
    if isinstance(error, ValueError) and "api_base is required" in message:
        return {"kind": "api_base_required", "message": ""}
    return {"kind": "failed", "message": ""}


async def call_provider(input_value: Mapping[str, Json], recorder: SleepRecorder) -> Json:
    request_id = cast(str, input_value.get("request_id", "request"))
    params = input_value.get("params", {})
    api_base = cast(str | None, input_value.get("api_base"))
    kwargs = cast(dict[str, Json], input_value.get("kwargs", {}))
    if input_value.get("mode") == "streaming":
        events = _PROVIDER.handle_streaming(request_id, params, api_base, **kwargs)
        async for event in events:
            recorder.steps.append({"event": event, "delay_ms": 0})
        return cast(Json, recorder.steps)
    return await _PROVIDER.handle_non_streaming(request_id, params, api_base, **kwargs)


def transform(input_value: Mapping[str, Json]) -> Json:
    script = cast(list[dict[str, Json]], copy.deepcopy(input_value.get("responses", [])))
    clients: list[ScriptedClient] = []

    def factory(**options: Json) -> ScriptedClient:
        params = cast(dict[str, Json], options["params"])
        client = ScriptedClient(script, params.get("timeout"))
        clients.append(client)
        return client

    recorder = SleepRecorder()
    with (
        patch.object(_TRANSFORMATION, "get_async_httpx_client", factory),
        patch.object(_TRANSFORMATION, "asyncio", recorder),
    ):
        try:
            outcome: Json = {"ok": asyncio.run(call_provider(input_value, recorder))}
        except Exception as error:  # noqa: BLE001
            outcome = {"error": classify(error)}
    requests: list[Json] = []
    for client in clients:
        requests.extend(client.requests)
    return {"outcome": outcome, "requests": requests, "sleeps": recorder.sleeps}


def run(input_value: dict[str, Json]) -> Json:
    ids = (UUID(int=value) for value in range(1, 100))
    with patch.object(_TRANSFORMATION, "uuid4", side_effect=ids):
        return normalize(transform(input_value), {})
