import asyncio
import re
from collections.abc import AsyncIterator, Callable, Mapping
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from typing import TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_TRANSFORMATION_PATH = (
    Path(litellm.__file__).parent
    / "a2a_protocol"
    / "providers"
    / "pydantic_ai_agents"
    / "transformation.py"
)
_SPEC = spec_from_file_location("b4805_litellm_transformation", _TRANSFORMATION_PATH)
if _SPEC is None or _SPEC.loader is None:
    raise ImportError(f"cannot load LiteLLM transformation at {_TRANSFORMATION_PATH}")
_MODULE = module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
_METHODS = cast(
    dict[str, object],
    vars(cast(dict[str, object], vars(_MODULE))["PydanticAITransformation"]),
)
_EXTRACT = cast(
    Callable[[Mapping[str, Json]], tuple[Json, Json, list[Json]]],
    _METHODS["_extract_response_text"],
)
_TRANSFORM = cast(
    Callable[[Mapping[str, Json], str], dict[str, Json]],
    _METHODS["_transform_to_a2a_response"],
)
_STREAM = cast(
    Callable[[Mapping[str, Json], str, int, int], AsyncIterator[dict[str, Json]]],
    _METHODS["fake_streaming_from_response"],
)
_COUNTER_ID = re.compile(r"^00000000-0000-0000-0000-[0-9a-f]{12}$")


class SleepRecorder:
    def __init__(self) -> None:
        self.steps: list[dict[str, Json]] = []

    async def sleep(self, seconds: float) -> None:
        last = self.steps[-1]
        last["delay_ms"] = cast(int, last["delay_ms"]) + round(seconds * 1000)


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


async def stream(
    response: Mapping[str, Json], request_id: str, chunk_size: int, delay_ms: int
) -> list[dict[str, Json]]:
    recorder = SleepRecorder()
    with patch.object(_MODULE, "asyncio", recorder):
        async for event in _STREAM(response, request_id, chunk_size, delay_ms):
            recorder.steps.append({"event": event, "delay_ms": 0})
    return recorder.steps


def transform(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    request_id = cast(str, input_value.get("request_id", "request"))
    response = cast(Mapping[str, Json], input_value.get("response", {}))
    if operation == "extract":
        return list(_EXTRACT(response))
    if operation == "transform":
        return cast(Json, _TRANSFORM(response, request_id))
    if operation == "stream":
        chunk_size = cast(int, input_value.get("chunk_size", 50))
        delay_ms = cast(int, input_value.get("delay_ms", 10))
        return cast(Json, asyncio.run(stream(response, request_id, chunk_size, delay_ms)))
    raise ValueError(f"unsupported B4805 operation: {operation}")


def run(input_value: dict[str, Json]) -> Json:
    ids = (UUID(int=value) for value in range(1, 100))
    with patch.object(_MODULE, "uuid4", side_effect=ids):
        return normalize(transform(input_value), {})
