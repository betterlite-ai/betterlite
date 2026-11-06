import asyncio
import sys
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from types import ModuleType
from typing import NamedTuple, TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_PACKAGE = "litellm.a2a_protocol.providers.watsonx_orchestrate"
_PROVIDER_DIR = Path(litellm.__file__).parent / "a2a_protocol" / "providers" / "watsonx_orchestrate"


def _load(name: str) -> ModuleType:
    # the a2a_protocol package needs the a2a SDK, so the provider files are loaded by path
    path = _PROVIDER_DIR / f"{name}.py"
    spec = spec_from_file_location(f"{_PACKAGE}.{name}", path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load LiteLLM watsonx Orchestrate module at {path}")
    module = module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


_TRANSFORMATION_MODULE = _load("transformation")
_HANDLER_MODULE = _load("handler")
_TRANSFORMATION = cast(
    dict[str, object],
    vars(cast(dict[str, object], vars(_TRANSFORMATION_MODULE))["WatsonxOrchestrateTransformation"]),
)
_HANDLER = cast(
    dict[str, object],
    vars(cast(dict[str, object], vars(_HANDLER_MODULE))["WatsonxOrchestrateHandler"]),
)
_PARAMS_TEXT = cast(
    Callable[[Mapping[str, Json]], str], _TRANSFORMATION["extract_text_from_a2a_params"]
)
_RESULT_TEXT = cast(Callable[[Json], str], _TRANSFORMATION["extract_text_from_wxo_result"])
_RESPONSE = cast(
    Callable[[str, str], dict[str, Json]], _TRANSFORMATION["build_a2a_message_response"]
)
_STREAM = cast(
    Callable[[str, str, int, int], AsyncIterator[dict[str, Json]]],
    _TRANSFORMATION["fake_streaming_from_text"],
)
_EXTRACT_CONFIG = cast(
    Callable[[Mapping[str, Json]], NamedTuple], _HANDLER["_extract_litellm_params"]
)
_BEARER_TOKEN = cast(Callable[..., Awaitable[str]], _HANDLER["_get_bearer_token"])
_TOKEN_CACHE = cast(dict[str, object], vars(_HANDLER_MODULE)["_token_cache"])


class FakeTime:
    def __init__(self, monotonic: float, wall: float) -> None:
        self._monotonic = monotonic
        self._wall = wall

    def monotonic(self) -> float:
        return self._monotonic

    def time(self) -> float:
        return self._wall


class FakeResponse:
    def __init__(self, body: Json) -> None:
        self._body = body

    def raise_for_status(self) -> None:
        return None

    def json(self) -> Json:
        return self._body


class FakeClient:
    def __init__(self, body: Json) -> None:
        self._body = body
        self.requests: list[Json] = []

    async def post(
        self,
        url: str,
        *,
        data: Json = None,
        json: Json = None,
        headers: Json = None,
    ) -> FakeResponse:
        self.requests.append({"url": url, "data": data, "json": json, "headers": headers})
        return FakeResponse(self._body)


class SleepRecorder:
    def __init__(self) -> None:
        self.steps: list[dict[str, Json]] = []

    async def sleep(self, seconds: float) -> None:
        last = self.steps[-1]
        last["delay_ms"] = cast(int, last["delay_ms"]) + round(seconds * 1000)


async def token_step(step: Mapping[str, Json]) -> Json:
    params = cast(Mapping[str, Json], step["params"])
    client = FakeClient(step.get("answer", {}))
    clock = FakeTime(cast(float, step["at"]), cast(float, step["wall"]))
    with patch.object(_HANDLER_MODULE, "time", clock):
        try:
            token = await _BEARER_TOKEN(
                cp4d_host=cast(str, params["cp4d_host"]),
                auth_mode=cast(str, params["auth_mode"]),
                api_key=cast(str, params["api_key"]),
                username=cast("str | None", params.get("username")),
                client=client,
            )
        except ValueError:
            return {"error": "missing_username"}
    return {"token": token, "requests": client.requests}


async def tokens(steps: list[Json]) -> Json:
    _TOKEN_CACHE.clear()
    return [await token_step(cast(Mapping[str, Json], step)) for step in steps]


async def stream(text: str, request_id: str, chunk_size: int, delay_ms: int) -> Json:
    recorder = SleepRecorder()
    with patch.object(_TRANSFORMATION_MODULE, "asyncio", recorder):
        async for event in _STREAM(text, request_id, chunk_size, delay_ms):
            recorder.steps.append({"event": event, "delay_ms": 0})
    return cast(Json, recorder.steps)


def transform(input_value: Mapping[str, Json]) -> Json:
    operation = input_value.get("operation")
    if operation == "config":
        params = cast(Mapping[str, Json], input_value["litellm_params"])
        return cast(Json, _EXTRACT_CONFIG(params)._asdict())
    if operation == "token":
        return asyncio.run(tokens(cast(list[Json], input_value["steps"])))
    if operation == "params_text":
        return _PARAMS_TEXT(cast(Mapping[str, Json], input_value["params"]))
    if operation == "result_text":
        return _RESULT_TEXT(input_value.get("result"))
    if operation == "response":
        return cast(
            Json,
            _RESPONSE(cast(str, input_value["request_id"]), cast(str, input_value["text"])),
        )
    if operation == "stream":
        return asyncio.run(
            stream(
                cast(str, input_value["text"]),
                cast(str, input_value.get("request_id", "request")),
                cast(int, input_value.get("chunk_size", 50)),
                cast(int, input_value.get("delay_ms", 10)),
            )
        )
    raise ValueError(f"unsupported B4807 operation: {operation}")


def run(input_value: dict[str, Json]) -> Json:
    ids = (UUID(int=value) for value in range(1, 100))
    with patch.object(_TRANSFORMATION_MODULE, "uuid4", side_effect=ids):
        return transform(input_value)
